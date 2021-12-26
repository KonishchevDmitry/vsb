use crate::config::BackupConfig;
use crate::core::GenericResult;
use crate::storage::Storage;

pub struct BackupFile {
    // storage: Storage, // FIXME(konishchev): Ref counter
}

// FIXME(konishchev): Cleanup on error
impl BackupFile {
    pub fn create(config: &BackupConfig, storage: Storage) -> GenericResult<BackupFile> {
        storage.create_backup(config.max_backups)?;
        Ok(BackupFile {})
    }
}