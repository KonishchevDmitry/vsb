use std::collections::{BTreeMap, BTreeSet};
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

    pub fn get_backup_groups(&self) -> GenericResult<BackupGroups> {
        let backup_group_re = Regex::new(r"^\d{4}\.\d{2}\.\d{2}$")?;

        let provider = self.provider.read();
        let mut backup_groups = BackupGroups::new();

        let files = provider.list_directory(&self.path)?.ok_or_else(|| format!(
            "Backup root {:?} doesn't exist", self.path))?;

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            let group_path = self.path.trim_right_matches('/').to_owned() + "/" + &file.name;

            match file.type_ {
                FileType::Directory if backup_group_re.is_match(&file.name) => {
                    backup_groups.entry(file.name.clone()).or_insert_with(Backups::new).extend(
                        get_backups(provider, &group_path)?.iter().cloned());
                },
                _ => {
                    error!("{:?} backup root on {} contains an unexpected {}: {:?}.",
                        self.path, provider.name(), file.type_, file.name);
                },
            };
        }

        Ok(backup_groups)
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
        let cloud_backup_path = self.get_backup_path(group_name, &backup_name);

        // FIXME: Stream size
        let (chunk_streams, splitter_thread) = stream_splitter::split(
            data_stream, 10 * 1024 * 1024)?;

        let archive_thread = match thread::Builder::new().name("backup archiving thread".into()).spawn(move || {
            archive_backup(&backup_name, &local_backup_path, encryptor)
        }) {
            Ok(handle) => handle,
            Err(err) => {
                util::join_thread_ignoring_result(splitter_thread);
                return Err!("Unable to spawn a thread: {}", err);
            },
        };

        let upload_result = provider.upload_file(&cloud_backup_path, chunk_streams);
        let archive_result = util::join_thread(archive_thread).map_err(|e| format!(
            "Archive operation has failed: {}", e));

        let splitter_result = util::join_thread(splitter_thread);

        // FIXME: Actually, no and on tar writting error we upload the broken backup
        // The real error should always be here, but...
        upload_result?;

        // ... just in case, check these results too, to not miss anything.
        archive_result?;
        splitter_result?;

        Ok(())
    }

    pub fn get_backup_group_path(&self, group_name: &str) -> String {
        self.path.trim_right_matches('/').to_owned() + "/" + group_name
    }

    pub fn get_backup_path(&self, group_name: &str, backup_name: &str) -> String {
        let backup_file_extension = match self.provider.read().type_() {
            ProviderType::Local => "",
            ProviderType::Cloud => CLOUD_BACKUP_FILE_EXTENSION,
        };
        self.get_backup_group_path(group_name) + "/" + backup_name + backup_file_extension
    }
}

const CLOUD_BACKUP_FILE_EXTENSION: &str = ".tar.gpg";

fn get_backups(provider: &ReadProvider, group_path: &str) -> GenericResult<Vec<String>> {
    let mut backups = Vec::new();

    let (backup_file_type, backup_file_extension) = match provider.type_() {
        ProviderType::Local => (FileType::Directory, ""),
        ProviderType::Cloud => (FileType::File, CLOUD_BACKUP_FILE_EXTENSION),
    };

    let backup_file_re: Regex = Regex::new(&(
        r"^(\d{4}\.\d{2}\.\d{2}-\d{2}:\d{2}:\d{2})".to_owned()
            + &regex::escape(backup_file_extension) + "$"))?;

    let files = provider.list_directory(group_path)?.ok_or_else(|| format!(
        "Failed to list {:?} backup group: it doesn't exist", group_path))?;

    for file in files {
        if file.name.starts_with('.') {
            continue
        }

        let captures = backup_file_re.captures(&file.name);

        if file.type_ != backup_file_type || captures.is_none() {
            error!("{:?} backup group on {} contains an unexpected {}: {:?}.",
                group_path, provider.name(), file.type_, file.name);
            continue
        }

        backups.push(captures.unwrap().get(1).unwrap().as_str().to_owned());
    }

    Ok(backups)
}

fn archive_backup(backup_name: &str, backup_path: &str, encryptor: Encryptor) -> EmptyResult {
    let mut archive = tar::Builder::new(encryptor);
    archive.append_dir_all(backup_name, backup_path)?;

    let encryptor = archive.into_inner()?;
    encryptor.finish()?;

    Ok(())
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