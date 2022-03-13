mod backup;
mod backuper;

use crate::config::BackupConfig;
use crate::core::GenericResult;
use crate::providers::filesystem::Filesystem;
use crate::storage::Storage;

pub use self::backup::BackupInstance;
pub use self::backuper::Backuper;

// FIXME(konishchev): Implement
pub fn backup(backup_config: &BackupConfig) -> GenericResult<bool> {
    let storage = Storage::new(Filesystem::new(), &backup_config.path);
    let (backup, ok) = BackupInstance::create(backup_config, storage)?;

    let backuper = Backuper::new(backup_config, backup, false)?;
    Ok(ok && backuper.run().is_ok())
}