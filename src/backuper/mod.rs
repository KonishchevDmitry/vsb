// FIXME(konishchev): Drop
#![allow(clippy::module_inception)]

mod backup;
mod backuper;
mod file_reader;

use crate::config::BackupConfig;
use crate::core::GenericResult;
use crate::providers::filesystem::Filesystem;
use crate::storage::Storage;

use self::backup::BackupFile;
use self::backuper::Backuper;

// FIXME(konishchev): Implement
pub fn backup(backup_config: &BackupConfig) -> GenericResult<bool> {
    let storage = Storage::new(Filesystem::new(), &backup_config.path);
    let backup = BackupFile::create(backup_config, storage)?;

    let backuper = Backuper::new(backup_config, backup)?;
    Ok(backuper.run().is_ok())
}