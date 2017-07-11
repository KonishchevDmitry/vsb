use std::collections::{BTreeMap, BTreeSet};

use regex::{self, Regex};

use core::{EmptyResult, GenericResult};
use provider::{ProviderType, ReadProvider, WriteProvider, FileType};

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

    fn get_backup_group_path(&self, group_name: &str) -> String {
        self.path.trim_right_matches('/').to_owned() + "/" + group_name
    }
}

const CLOUD_BACKUP_EXTENSION: &str = ".tar.gpg";

fn get_backups(provider: &ReadProvider, group_path: &str) -> GenericResult<Vec<String>> {
    let mut backups = Vec::new();

    let (backup_file_type, backup_file_extension) = match provider.type_() {
        ProviderType::Local => (FileType::Directory, ""),
        ProviderType::Cloud => (FileType::File, CLOUD_BACKUP_EXTENSION),
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

pub type BackupGroups = BTreeMap<String, Backups>;
pub type Backups = BTreeSet<String>;

// Rust don't have trait upcasting yet (https://github.com/rust-lang/rust/issues/5665), so we have
// to emulate it via this trait.
trait AbstractProvider {
    fn read(&self) -> &ReadProvider;
    fn write(&mut self) -> GenericResult<&WriteProvider>;
}

struct ReadOnlyProviderAdapter<T: ReadProvider> {
    provider: T,
}

impl<T: ReadProvider> AbstractProvider for ReadOnlyProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&mut self) -> GenericResult<&WriteProvider> {
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

    fn write(&mut self) -> GenericResult<&WriteProvider> {
        Ok(&self.provider)
    }
}