mod backup;
mod backuper;

use crate::config::BackupConfig;
use crate::core::GenericResult;
use crate::providers::filesystem::Filesystem;
use crate::storage::Storage;

use self::backup::BackupFile;
use self::backuper::Backuper;

pub fn backup(backup_config: &BackupConfig) -> GenericResult<bool> {
    let storage = Storage::new(Filesystem::new(), &backup_config.path);
    let _backup = BackupFile::create(backup_config, storage)?;

    let mut backuper = Backuper::new(backup_config)?;
    let ok = backuper.run();

    Ok(ok)
}