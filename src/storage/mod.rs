mod adapters;
mod backup;
mod backup_group;
mod helpers;

use std::time::SystemTime;

use chrono::{self, TimeZone};
use rayon::prelude::*;

use crate::core::{EmptyResult, GenericResult};
use crate::encryptor::Encryptor;
use crate::provider::{ReadProvider, WriteProvider};
use crate::stream_splitter;
use crate::util;

use self::adapters::{AbstractProvider, ReadOnlyProviderAdapter, ReadWriteProviderAdapter};
use self::helpers::BackupFileTraits;

pub use self::backup::Backup;
pub use self::backup_group::BackupGroup;

pub struct Storage {
    provider: Box<dyn AbstractProvider>,
    path: String,
}

impl Storage {
    pub fn new<T: ReadProvider + WriteProvider + 'static>(provider: T, path: &str) -> Storage {
        Storage {
            provider: ReadWriteProviderAdapter::new(provider),
            path: path.to_owned(),
        }
    }

    pub fn new_read_only<T: ReadProvider +'static>(provider: T, path: &str) -> Storage {
        Storage {
            provider: ReadOnlyProviderAdapter::new(provider),
            path: path.to_owned(),
        }
    }

    pub fn name(&self) -> &str {
        self.provider.read().name()
    }

    pub fn get_backup_groups(&self, verify: bool) -> GenericResult<(Vec<BackupGroup>, bool)> {
        let provider = self.provider.read();
        let (mut groups, mut ok) = BackupGroup::list(provider, &self.path)?;

        if verify && !groups.is_empty() {
            info!("Verifying backups on {}...", self.name());
            ok &= groups.par_iter_mut().map(|group| {
                group.inspect(provider)
            }).all(|result| result);
        }

        Ok((groups, ok))
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
        self.path.trim_end_matches('/').to_owned() + "/" + group_name
    }

    pub fn get_backup_path(&self, group_name: &str, backup_name: &str) -> String {
        self.get_backup_group_path(group_name) + "/" + &self.get_backup_file_name(backup_name, false)
    }

    fn get_backup_file_name(&self, backup_name: &str, temporary: bool) -> String {
        let extension = BackupFileTraits::get_for(self.provider.read().type_()).extension;

        let prefix = if temporary {
            "."
        } else {
            ""
        }.to_owned();

        prefix + backup_name + extension
    }

    pub fn get_backup_time(&self, backup_name: &str) -> GenericResult<SystemTime> {
        let backup_time = chrono::offset::Local.datetime_from_str(backup_name, "%Y.%m.%d-%H:%M:%S")
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