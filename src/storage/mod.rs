mod adapters;
mod backup;
mod backup_group;
mod encryptor;
pub mod metadata;
mod traits;

use std::rc::Rc;
use std::time::SystemTime;

use chrono::{self, offset::Local, TimeZone};
use log::info;
use rayon::prelude::*;

use crate::core::{EmptyResult, GenericResult};
use crate::providers::{FileType, ReadProvider, WriteProvider};
use crate::util::{self, stream_splitter};

use self::adapters::{AbstractProvider, ReadOnlyProviderAdapter, ReadWriteProviderAdapter};
use self::encryptor::Encryptor;

pub use self::backup::Backup;
pub use self::backup_group::BackupGroup;
pub use self::traits::BackupTraits;

pub type StorageRc = Rc<Storage>;

pub struct Storage {
    pub provider: Box<dyn AbstractProvider>,
    path: String,
}

impl Storage {
    pub fn new<T: ReadProvider + WriteProvider + 'static>(provider: T, path: &str) -> StorageRc {
        Rc::new(Storage {
            provider: ReadWriteProviderAdapter::new(provider),
            path: path.to_owned(),
        })
    }

    pub fn new_read_only<T: ReadProvider + 'static>(provider: T, path: &str) -> StorageRc {
        Rc::new(Storage {
            provider: ReadOnlyProviderAdapter::new(provider),
            path: path.to_owned(),
        })
    }

    pub fn name(&self) -> &str {
        self.provider.read().name()
    }

    pub fn backup_traits(&self) -> &'static BackupTraits {
        BackupTraits::get_for(self.provider.read().type_())
    }

    // FIXME(konishchev): Rewrite
    fn clarification(&self) -> String {
        format!(" on {}", self.name())
    }

    pub fn get_backup_groups(&self, verify: bool) -> GenericResult<(Vec<BackupGroup>, bool)> {
        let provider = self.provider.read();
        let (mut groups, mut ok) = BackupGroup::list(provider, &self.path)?;

        if verify && !groups.is_empty() {
            info!("Verifying backups on {}...", self.name());
            ok &= groups.par_iter_mut().map(|group: &mut BackupGroup| {
                group.inspect(provider)
            }).all(|result| result);
        }

        Ok((groups, ok))
    }

    pub fn create_backup_group(&self, name: &str) -> GenericResult<BackupGroup> {
        info!("Creating {:?} backup group{}...", name, self.clarification());
        let path = self.get_backup_group_path(name);
        self.provider.write()?.create_directory(&path)?;
        Ok(BackupGroup::new(name))
    }

    pub fn get_backup_group(&self, name: &str, strict: bool) -> GenericResult<BackupGroup> {
        let provider = self.provider.read();
        let path = self.get_backup_group_path(name);
        let (group, _ok) = BackupGroup::read(provider, name, &path, strict).map_err(|e| format!(
            "Failed to read {:?} backup group: {}", path, e))?;
        Ok(group)
    }

    pub fn create_backup(&self, max_backups: usize) -> GenericResult<(BackupGroup, Backup)> {
        let provider = self.provider.write()?;

        let backup_traits = self.backup_traits();
        if backup_traits.file_type != FileType::Directory {
            return Err!("Backup creation is not supported for {} provider", provider.name());
        }

        let now = Local::now();
        let (mut groups, _ok) = self.get_backup_groups(false)?;

        let group = match groups.last() {
            Some(group) if group.backups.len() < max_backups => {
                // FIXME(konishchev): Cleanup group from temporary files?
                info!("Using {:?} backup group{}.", group.name, self.clarification());
                groups.pop().unwrap()
            },
            _ => {
                let group_name = now.format(backup_traits.group_name_format).to_string();
                if groups.iter().any(|group| group.name == group_name) {
                    return Err!("Unable to create new backup group ({}): it already exists", group_name);
                }
                self.create_backup_group(&group_name)?
            },
        };

        // FIXME(konishchev): Check existence?
        let backup_name = now.format(backup_traits.name_format).to_string();
        let backup_path = self.get_backup_path(&group.name, &backup_name, true);
        let backup = Backup::new(&backup_path, &backup_name);

        info!("Creating {:?} backup{}...", backup.name, self.clarification());
        provider.create_directory(&backup.path)?;

        // FIXME(konishchev): Add to group?
        Ok((group, backup))
    }

    pub fn upload_backup(&self, local_backup_path: &str, group_name: &str, backup_name: &str,
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

        let archive_thread = match util::sys::spawn_thread("backup archiver", move || {
            archive_backup(&backup_name, &local_backup_path, encryptor)
        }) {
            Ok(handle) => handle,
            Err(err) => {
                util::sys::join_thread_ignoring_result(splitter_thread);
                return Err(err);
            }
        };

        let upload_result = provider.upload_file(
            &group_path, &temp_file_name, &file_name, chunk_streams);

        let archive_result = util::sys::join_thread(archive_thread).map_err(|e| format!(
            "Archive operation has failed: {}", e));

        let splitter_result = util::sys::join_thread(splitter_thread);

        // The real error should always be here, but...
        upload_result?;

        // ... just in case, check these results too, to not miss anything.
        archive_result?;
        splitter_result?;

        Ok(())
    }

    pub fn delete_backup_group(&self, group_name: &str) -> EmptyResult {
        let group_path = self.get_backup_group_path(group_name);
        self.provider.write()?.delete(&group_path)
    }

    pub fn get_backup_group_path(&self, group_name: &str) -> String {
        self.path.trim_end_matches('/').to_owned() + "/" + group_name
    }

    pub fn get_backup_path(&self, group_name: &str, backup_name: &str, temporary: bool) -> String {
        self.get_backup_group_path(group_name) + "/" + &self.get_backup_file_name(backup_name, temporary)
    }

    fn get_backup_file_name(&self, backup_name: &str, temporary: bool) -> String {
        let extension = BackupTraits::get_for(self.provider.read().type_()).extension;

        let prefix = if temporary {
            "."
        } else {
            ""
        }.to_owned();

        prefix + backup_name + extension
    }

    pub fn get_backup_time(&self, backup_name: &str) -> GenericResult<SystemTime> {
        let backup_time = Local.datetime_from_str(backup_name, self.backup_traits().name_format)
            .map_err(|_| format!("Invalid backup name: {:?}", backup_name))?;

        Ok(SystemTime::from(backup_time))
    }
}

fn archive_backup(backup_name: &str, backup_path: &str, encryptor: Encryptor) -> EmptyResult {
    let mut archive = tar::Builder::new(encryptor);

    if let Err(err) = archive.append_dir_all(backup_name, backup_path) {
        let _ = archive.finish();
        return Err(archive.into_inner().unwrap().finish(Some(err.to_string())).unwrap_err());
    }

    if let Err(err) = archive.finish() {
        return Err(archive.into_inner().unwrap().finish(Some(err.to_string())).unwrap_err());
    }

    archive.into_inner().unwrap().finish(None)
}
