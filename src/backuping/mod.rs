mod backup;
mod backuper;
mod file_reader;

use crate::config::BackupConfig;
use crate::core::GenericResult;
use crate::providers::filesystem::Filesystem;
use crate::storage::Storage;

use self::backup::BackupInstance;
use self::backuper::Backuper;

// FIXME(konishchev): Implement
pub fn backup(backup_config: &BackupConfig) -> GenericResult<bool> {
    let storage = Storage::new(Filesystem::new(), &backup_config.path);
    let backup = BackupInstance::create(backup_config, storage)?;

    let backuper = Backuper::new(backup_config, backup, false)?;
    Ok(backuper.run().is_ok())
}

#[cfg(test)]
mod test {
    use std::fs;

    use assert_fs::fixture::TempDir;

    use crate::config::BackupItemConfig;
    use crate::core::EmptyResult;
    use crate::metadata::MetadataItem;

    use super::*;

    // FIXME(konishchev): Rewrite
    #[test]
    fn backup() -> EmptyResult {
        easy_logging::init(module_path!().split("::").next().unwrap(), log::Level::Warn)?;

        let temp_dir = TempDir::new()?;
        let backup_root_path = temp_dir.join("backups");
        fs::create_dir(&backup_root_path)?;

        let root_path = std::env::current_dir()?.join("src");

        let backup_config = BackupConfig {
            name: s!("test"),
            path: backup_root_path.to_str().unwrap().to_owned(),
            items: Some(vec![BackupItemConfig {
                path: root_path.to_str().unwrap().to_owned(),
            }]),
            max_backups: 100,
            upload: None
        };

        let filesystem = Filesystem::new();
        let storage = Storage::new(Filesystem::new(), backup_root_path.to_str().unwrap());
        let backup = BackupInstance::create(&backup_config, storage)?;
        let backuper = Backuper::new(&backup_config, backup, true)?;
        assert!(backuper.run().is_ok());

        let storage = Storage::new(Filesystem::new(), backup_root_path.to_str().unwrap());
        let (groups, ok) = storage.get_backup_groups(true)?;
        assert!(ok);
        assert_eq!(groups.len(), 1);

        let group = groups.last().unwrap();
        assert_eq!(group.backups.len(), 1);

        let backup = group.backups.last().unwrap();
        let files = backup.read_metadata(&filesystem)?.into_iter()
            .collect::<GenericResult<Vec<MetadataItem>>>()?;
        assert!(!files.is_empty());

        for file in files {
            assert!(file.unique, "{} is not unique", file.path);
        }

        Ok(())
    }
}