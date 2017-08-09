use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{BufRead, BufReader};
use std::time::SystemTime;

use bzip2::read::BzDecoder;
use regex::{self, Regex};
use tar;

use chrono::{self, TimeZone};
use core::{EmptyResult, GenericResult};
use encryptor::Encryptor;
use provider::{ProviderType, ReadProvider, WriteProvider, FileType};
use stream_splitter;
use util;

pub struct Storage {
    provider: Box<AbstractProvider>,
    path: String,
}

impl Storage {
    pub fn new<T: ReadProvider + WriteProvider + 'static>(provider: T, path: &str) -> Storage {
        Storage {
            provider: Box::new(ReadWriteProviderAdapter{provider: provider}),
            path: path.to_owned(),
        }
    }

    pub fn new_read_only<T: ReadProvider +'static>(provider: T, path: &str) -> Storage {
        Storage {
            provider: Box::new(ReadOnlyProviderAdapter{provider: provider}),
            path: path.to_owned(),
        }
    }

    pub fn name(&self) -> &str {
        self.provider.read().name()
    }

    pub fn get_backup_groups(&self) -> GenericResult<(BackupGroups, bool)> {
        let mut ok = true;
        let backup_group_re = Regex::new(r"^\d{4}\.\d{2}\.\d{2}$")?;

        let provider = self.provider.read();
        let mut backup_groups = BackupGroups::new();

        let files = provider.list_directory(&self.path)?
            .ok_or_else(|| format!("{:?} backup root doesn't exist", self.path))?;

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            if file.type_ != FileType::Directory || !backup_group_re.is_match(&file.name) {
                error!("{:?} backup root on {} contains an unexpected {}: {:?}.",
                    self.path, provider.name(), file.type_, file.name);
                ok = false;
                continue;
            }

            let group_name = &file.name;
            let group_path = self.path.trim_right_matches('/').to_owned() + "/" + group_name;

            let (backups, mut backups_ok) = get_backups(provider, &group_path).map_err(|e| format!(
                "Unable to list {:?} backup group: {}", group_path, e))?;

            let mut backup_group = backup_groups.entry(group_name.to_owned())
                .or_insert_with(Backups::new);

            if !backups.is_empty() {
                backup_group.extend(backups.iter().cloned());
                let first_backup_name = backup_group.iter().next().unwrap();

                if first_backup_name.split("-").next().unwrap() != group_name {
                    error!(concat!(
                        "Suspicious first backup ({:?}) in {:?} group on {}: ",
                        "a possibly corrupted backup group."
                    ), first_backup_name, group_name, provider.name());
                    backups_ok = false;
                }
            }

            ok &= backups_ok;
        }

        Ok((backup_groups, ok))
    }

    pub fn create_backup_group(&mut self, group_name: &str) -> EmptyResult {
        let group_path = self.get_backup_group_path(group_name);
        self.provider.write()?.create_directory(&group_path)
    }

    pub fn upload_backup(&mut self, local_backup_path: &str, group_name: &str, backup_name: &str,
                         encryption_passphrase: &str) -> EmptyResult {
        let provider = self.provider.write()?;
        let (encryptor, data_stream) = Encryptor::new(encryption_passphrase, provider.hasher())?;

        let backup_name = backup_name.to_owned();
        let local_backup_path = local_backup_path.to_owned();
        let group_path = self.get_backup_group_path(group_name);
        let temp_file_name = self.get_backup_file_name(&backup_name, true);
        let file_name = self.get_backup_file_name(&backup_name, false);

        let (chunk_streams, splitter_thread) = stream_splitter::split(
            data_stream, provider.max_request_size())?;

        let archive_thread = match util::spawn_thread("backup archiver", move || {
            archive_backup(&backup_name, &local_backup_path, encryptor)
        }) {
            Ok(handle) => handle,
            Err(err) => {
                util::join_thread_ignoring_result(splitter_thread);
                return Err(err);
            }
        };

        let upload_result = provider.upload_file(
            &group_path, &temp_file_name, &file_name, chunk_streams);

        let archive_result = util::join_thread(archive_thread).map_err(|e| format!(
            "Archive operation has failed: {}", e));

        let splitter_result = util::join_thread(splitter_thread);

        // The real error should always be here, but...
        upload_result?;

        // ... just in case, check these results too, to not miss anything.
        archive_result?;
        splitter_result?;

        Ok(())
    }

    pub fn delete_backup_group(&mut self, group_name: &str) -> EmptyResult {
        let group_path = self.get_backup_group_path(group_name);
        self.provider.write()?.delete(&group_path)
    }

    pub fn get_backup_group_path(&self, group_name: &str) -> String {
        self.path.trim_right_matches('/').to_owned() + "/" + group_name
    }

    pub fn get_backup_path(&self, group_name: &str, backup_name: &str) -> String {
        self.get_backup_group_path(group_name) + "/" + &self.get_backup_file_name(backup_name, false)
    }

    fn get_backup_file_name(&self, backup_name: &str, temporary: bool) -> String {
        let extension = BackupFileTraits::get_for(self.provider.read().type_()).extension;

        let prefix = match temporary {
            true => ".",
            false => "",
        }.to_owned();

        prefix + backup_name + extension
    }

    pub fn get_backup_time(&self, backup_name: &str) -> GenericResult<SystemTime> {
        let backup_time = chrono::offset::Local.datetime_from_str(&backup_name, "%Y.%m.%d-%H:%M:%S")
            .map_err(|_| format!("Invalid backup name: {:?}", backup_name))?;

        Ok(SystemTime::from(backup_time))
    }
}

pub type BackupGroups = BTreeMap<String, Backups>;
pub type Backups = BTreeSet<String>;

struct BackupFileTraits {
    type_: FileType,
    extension: &'static str,
    name_re: Regex,
}

impl BackupFileTraits {
    fn get_for(provider_type: ProviderType) -> &'static BackupFileTraits {
        lazy_static! {
            static ref LOCAL_TRAITS: BackupFileTraits = BackupFileTraits {
                type_: FileType::Directory,
                extension: "",
                name_re: BackupFileTraits::get_name_re(""),
            };

            static ref CLOUD_TRAITS: BackupFileTraits = BackupFileTraits {
                type_: FileType::File,
                extension: ".tar.gpg",
                name_re: BackupFileTraits::get_name_re(".tar.gpg"),
            };
        }

        match provider_type {
            ProviderType::Local => &LOCAL_TRAITS,
            ProviderType::Cloud => &CLOUD_TRAITS,
        }
    }

    fn get_name_re(extension: &str) -> Regex {
        let regex = r"^(\d{4}\.\d{2}\.\d{2}-\d{2}:\d{2}:\d{2})".to_owned()
            + &regex::escape(extension) + "$";
        Regex::new(&regex).unwrap()
    }
}

fn get_backups(provider: &ReadProvider, group_path: &str) -> GenericResult<(Vec<String>, bool)> {
    let (mut backups, mut ok) = (Vec::new(), true);

    let backup_file_traits = BackupFileTraits::get_for(provider.type_());
    let mut files = provider.list_directory(group_path)?.ok_or_else(||
        "the backup group doesn't exist".to_owned())?;

    let mut available_checksums = HashSet::new();
    files.sort_by(|a, b| a.name.cmp(&b.name));

    for file in files {
        if file.name.starts_with('.') {
            continue
        }

        let captures = backup_file_traits.name_re.captures(&file.name);

        if file.type_ != backup_file_traits.type_ || captures.is_none() {
            error!("{:?} backup group on {} contains an unexpected {}: {:?}.",
                group_path, provider.name(), file.type_, file.name);
            ok = false;
            continue
        }

        let backup_name = captures.unwrap().get(1).unwrap().as_str().to_owned();

        if file.type_ == FileType::Directory {
            let backup_path = group_path.to_owned() + "/" + &file.name;

            match validate_backup(provider, &mut available_checksums, &backup_name, &backup_path) {
                Ok(recoverable) => ok = ok && recoverable,
                Err(err) => {
                    error!("{:?} backup on {} validation error: {}.",
                           &backup_path, provider.name(), err);
                    ok = false;
                    continue
                }
            };
        }

        backups.push(backup_name);
    }

    Ok((backups, ok))
}

fn validate_backup(provider: &ReadProvider, available_checksums: &mut HashSet<String>,
                   backup_name: &str, backup_path: &str) -> GenericResult<bool> {
    let metadata_name = "metadata.bz2";
    let metadata_files: HashSet<String> = [metadata_name].iter().map(|&s| s.to_owned()).collect();
    let data_files: HashSet<String> = ["data.tar.gz", "data.tar.bz2", "data.tar.7z"].iter()
        .map(|&s| s.to_owned()).collect();

    let mut backup_files = provider.list_directory(&backup_path)?.ok_or_else(||
        "the backup doesn't exist".to_owned())?;

    let backup_files: HashSet<String> = backup_files.drain(..).map(|file| file.name).collect();

    if backup_files.is_disjoint(&metadata_files) {
        return Err!("the backup is corrupted: metadata file is missing")
    }

    if backup_files.is_disjoint(&data_files) {
        return Err!("the backup is corrupted: backup data file is missing")
    }

    Ok(match provider.type_() {
        ProviderType::Local => {
            let metadata_path = backup_path.to_owned() + "/" + metadata_name;
            check_backup_consistency(provider, available_checksums, backup_name, &metadata_path)?
        },
        ProviderType::Cloud => true,
    })
}

fn check_backup_consistency(provider: &ReadProvider, available_checksums: &mut HashSet<String>,
                            backup_name: &str, metadata_path: &str) -> GenericResult<bool> {
    if cfg!(debug_assertions) {
        warn!("Skip consistency check of {:?}: running in develop mode.", metadata_path);
        return Ok(true);
    }

    let metadata_file = provider.open_file(metadata_path).map(BzDecoder::new).map(BufReader::new)
        .map_err(|e| format!("Unable to open metadata file: {}", e))?;

    let mut files = 0;

    for line in metadata_file.lines() {
        let line = line.map_err(|e| format!("Error while reading metadata file: {}", e))?;
        files += 1;

        let mut parts = line.splitn(4, " ");
        let checksum = parts.next();
        let status = parts.next();
        let fingerprint = parts.next();
        let filename = parts.next();

        let (checksum, unique, filename) = match (checksum, status, fingerprint, filename) {
            (Some(checksum), Some(status), Some(_), Some(filename))
                if status == "extern" || status == "unique" =>
                    (checksum.to_owned(), status == "unique", filename),
            _ => return Err!("Error while reading metadata file: it has an unsupported format"),
        };

        if unique {
            available_checksums.insert(checksum);
        } else if !available_checksums.contains(&checksum) {
            error!(concat!(
                "{:?} backup on {} is not recoverable: ",
                "unable to find extern {:?} file in the backup group."),
                   backup_name, provider.name(), filename);
            return Ok(false)
        }
    }

    if files == 0 {
        error!("{:?} backup on {} don't have any files.", backup_name, provider.name());
        return Ok(false)
    }

    Ok(true)
}

fn archive_backup(backup_name: &str, backup_path: &str, encryptor: Encryptor) -> EmptyResult {
    let mut archive = tar::Builder::new(encryptor);

    if let Err(err) = archive.append_dir_all(backup_name, backup_path) {
        let _ = archive.finish();
        return Err(archive.into_inner().unwrap().finish(Some(err.to_string())).unwrap_err().into());
    }

    if let Err(err) = archive.finish() {
        return Err(archive.into_inner().unwrap().finish(Some(err.to_string())).unwrap_err().into());
    }

    archive.into_inner().unwrap().finish(None)
}

// Rust don't have trait upcasting yet (https://github.com/rust-lang/rust/issues/5665), so we have
// to emulate it via this trait.
trait AbstractProvider {
    fn read(&self) -> &ReadProvider;
    fn write(&self) -> GenericResult<&WriteProvider>;
}

struct ReadOnlyProviderAdapter<T: ReadProvider> {
    provider: T,
}

impl<T: ReadProvider> AbstractProvider for ReadOnlyProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&self) -> GenericResult<&WriteProvider> {
        Err!("An attempt to modify a read-only backup storage")
    }
}

struct ReadWriteProviderAdapter<T: ReadProvider + WriteProvider> {
    provider: T,
}

impl<T: ReadProvider + WriteProvider> AbstractProvider for ReadWriteProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&self) -> GenericResult<&WriteProvider> {
        Ok(&self.provider)
    }
}
