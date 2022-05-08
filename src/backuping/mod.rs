mod backup;
mod backuper;
mod config;
mod filter;

use log::{info, error};

use crate::config::BackupSpecConfig;
use crate::core::GenericResult;
use crate::providers::filesystem::Filesystem;
use crate::storage::Storage;
use crate::util::sys::acquire_lock;

use self::backup::BackupInstance;
use self::backuper::Backuper;

pub use self::config::{BackupConfig, BackupItemConfig};
pub use self::filter::PathFilter;

pub fn backup(config: &BackupSpecConfig) -> GenericResult<bool> {
    let _lock = acquire_lock(&config.path)?;
    let storage = Storage::new_read_write(Filesystem::new(), &config.path);

    let config = config.backup.as_ref().ok_or(
        "Backup rules aren't configured for the specified backup")?;

    let (backup, mut ok) = BackupInstance::create(config, &storage)?;
    ok &= Backuper::new(config, backup)?.run()?;

    ok &= gc_groups(&storage, config.max_backup_groups)?;
    Ok(ok)
}

fn gc_groups(storage: &Storage, max_groups: usize) -> GenericResult<bool> {
    let (groups, mut ok) = storage.get_backup_groups(false)?;
    if groups.len() <= max_groups {
        return Ok(ok);
    }

    if !ok {
        error!("Do not remove old backup groups due to errors above.");
        return Ok(ok);
    }

    for group in &groups[..groups.len() - max_groups] {
        info!("Deleting {:?} backup group...", group.name);
        if let Err(err) = storage.delete_backup_group(&group.name) {
            error!("Failed to delete {:?} backup group: {}.", group.name, err);
            ok = false;
        }
    }

    Ok(ok)
}