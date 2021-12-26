use std::fs::File;
use std::path::Path;

use rayon::prelude::*;

use crate::config::BackupConfig;
use crate::core::GenericResult;
use crate::metadata::MetadataWriter;
use crate::storage::{Storage, Backup};

pub struct BackupFile {
    // storage: Storage, // FIXME(konishchev): Ref counter
    #[allow(dead_code)] // FIXME(konishchev): Drop
    metadata: MetadataWriter,
}

// FIXME(konishchev): Cleanup on error
impl BackupFile {
    pub fn create(config: &BackupConfig, storage: Storage) -> GenericResult<BackupFile> {
        let (group, backup) = storage.create_backup(config.max_backups)?;

        // FIXME(konishchev): Load metadata
        group.backups.par_iter().enumerate().for_each(|(_index, _backup): (usize, &Backup)| {
        });

        let metadata_path = Path::new(&backup.path).join(Backup::METADATA_NAME);
        let metadata = MetadataWriter::new(File::create(metadata_path)?);

        Ok(BackupFile {metadata})
    }
}