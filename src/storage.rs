use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::thread;

use regex::{self, Regex};
use tar;

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

        let files = provider.list_directory(&self.path)
            .map_err(|e| format!("Unable to list {:?} backup root on {}: {}",
                                 &self.path, provider.name(), e))?
            .ok_or_else(|| format!("Backup root {:?} doesn't exist", self.path))?;

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            match file.type_ {
                FileType::Directory if backup_group_re.is_match(&file.name) => {},
                _ => {
                    error!("{:?} backup root on {} contains an unexpected {}: {:?}.",
                        self.path, provider.name(), file.type_, file.name);
                    ok = false;
                    continue;
                },
            };

            let group_name = &file.name;
            let group_path = self.path.trim_right_matches('/').to_owned() + "/" + group_name;

            let (backups, mut backups_ok) = get_backups(provider, &group_path).map_err(|e| format!(
                "Unable to list {:?} backup group on {}: {}", group_path, provider.name(), e))?;

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
        let cloud_backup_temp_path = self.get_backup_path(group_name, &backup_name, true);
        let cloud_backup_path = self.get_backup_path(group_name, &backup_name, false);

        let (chunk_streams, splitter_thread) = stream_splitter::split(
            data_stream, provider.max_request_size())?;

        let archive_thread = match thread::Builder::new().name("backup archiving thread".into()).spawn(move || {
            archive_backup(&backup_name, &local_backup_path, encryptor)
        }) {
            Ok(handle) => handle,
            Err(err) => {
                util::join_thread_ignoring_result(splitter_thread);
                return Err!("Unable to spawn a thread: {}", err);
            },
        };

        let upload_result = provider.upload_file(
            &cloud_backup_temp_path, &cloud_backup_path, chunk_streams);

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

    pub fn get_backup_path(&self, group_name: &str, backup_name: &str, temporary: bool) -> String {
        let backup_file_extension = match self.provider.read().type_() {
            ProviderType::Local => "",
            ProviderType::Cloud => CLOUD_BACKUP_FILE_EXTENSION,
        };

        let mut path = self.get_backup_group_path(group_name) + "/";
        if temporary {
            path += ".";
        }
        path + backup_name + backup_file_extension
    }
}

const CLOUD_BACKUP_FILE_EXTENSION: &str = ".tar.gpg";

fn get_backups(provider: &ReadProvider, group_path: &str) -> GenericResult<(Vec<String>, bool)> {
    let mut backups = Vec::new();
    let mut ok = true;

    let (backup_file_type, backup_file_extension) = match provider.type_() {
        ProviderType::Local => (FileType::Directory, ""),
        ProviderType::Cloud => (FileType::File, CLOUD_BACKUP_FILE_EXTENSION),
    };

    let backup_file_re: Regex = Regex::new(&(
        r"^(\d{4}\.\d{2}\.\d{2}-\d{2}:\d{2}:\d{2})".to_owned()
            + &regex::escape(backup_file_extension) + "$"))?;

    let files = provider.list_directory(group_path)?.ok_or_else(||
        "the backup group doesn't exist".to_owned())?;

    for file in files {
        if file.name.starts_with('.') {
            continue
        }

        let captures = backup_file_re.captures(&file.name);

        if file.type_ != backup_file_type || captures.is_none() {
            error!("{:?} backup group on {} contains an unexpected {}: {:?}.",
                group_path, provider.name(), file.type_, file.name);
            ok = false;
            continue
        }

        if backup_file_type == FileType::Directory {
            let backup_path = group_path.to_owned() + "/" + &file.name;
            if let Err(err) = validate_backup(provider, &backup_path) {
                error!("{:?} backup on {} validation error: {}", &backup_path, provider.name(), err);
                ok = false;
                continue
            }
        }

        backups.push(captures.unwrap().get(1).unwrap().as_str().to_owned());
    }

    Ok((backups, ok))
}

fn validate_backup(provider: &ReadProvider, backup_path: &str) -> EmptyResult {
    let metadata_files: HashSet<String> = ["metadata.bz2"].iter().map(|&s| s.to_owned()).collect();
    let data_files: HashSet<String> = ["data.tar.gz", "data.tar.bz2", "data.tar.7z"].iter()
        .map(|&s| s.to_owned()).collect();

    let backup_files = provider.list_directory(&backup_path)?.ok_or_else(||
        "the backup doesn't exist".to_owned())?;

    let backup_files: HashSet<String> = backup_files.iter().map(|file| file.name.clone()).collect();

    if backup_files.is_disjoint(&metadata_files) {
        return Err!("the backup is corrupted: metadata file is missing")
    }

    if backup_files.is_disjoint(&data_files) {
        return Err!("the backup is corrupted: backup data file is missing")
    }

    Ok(())
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

pub type BackupGroups = BTreeMap<String, Backups>;
pub type Backups = BTreeSet<String>;

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