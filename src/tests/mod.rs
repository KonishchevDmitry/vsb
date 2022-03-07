use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use assert_fs::fixture::TempDir;
use digest::Digest;
use log::info;
use sha2::Sha512;

use crate::backuping::{Backuper, BackupInstance};
use crate::config::{BackupConfig, BackupItemConfig};
use crate::core::{GenericResult, EmptyResult};
use crate::hash::Hash;
use crate::metadata::{Fingerprint, MetadataItem};
use crate::providers::filesystem::Filesystem;
use crate::storage::{Backup, Storage};

// FIXME(konishchev): Rewrite
#[test]
fn backup() -> EmptyResult {
    easy_logging::init(module_path!().split("::").next().unwrap(), log::Level::Info)?;

    let temp_dir = TempDir::new()?;
    let backup_root_path = temp_dir.join("backups");
    fs::create_dir(&backup_root_path)?;

    let root_path = std::env::current_dir()?.join("src/tests/testdata/root");
    let user_path = root_path.join("home/user");
    let mutable_file_path = user_path.join("mutable");

    let backups_per_group = 3;
    let backup_config = BackupConfig {
        name: s!("test"),
        path: backup_root_path.to_str().unwrap().to_owned(),
        items: Some(vec![BackupItemConfig {
            path: root_path.to_str().unwrap().to_owned(),
        }]),
        max_backups: backups_per_group,
        upload: None
    };

    let filesystem = Filesystem::new();
    let storage = Storage::new(Filesystem::new(), backup_root_path.to_str().unwrap());

    for pass in 0..5 {
        info!("#{} pass...", pass);

        fs::write(&mutable_file_path, format!("pass-{}", pass))?;

        let (backup, ok) = BackupInstance::create(&backup_config, storage.clone())?;
        assert!(ok);

        let backuper = Backuper::new(&backup_config, backup, true)?;
        assert!(backuper.run().is_ok());

        let (groups, ok) = storage.get_backup_groups(true)?;
        assert!(ok);
        assert_eq!(groups.len(), pass / backups_per_group + 1);

        let group = groups.last().unwrap();
        assert_eq!(group.backups.len(), pass % backups_per_group + 1);

        let backup = group.backups.last().unwrap();
        let files = read_metadata(&filesystem, backup)?;

        for empty_file in [user_path.join("empty"), user_path.join("other-empty")] {
            assert!(empty_file.exists());
            assert!(!files.contains_key(&empty_file));
        }
        assert!(files.contains_key(&user_path.join("non-empty")));

        for (path, file) in files {
            let metadata = fs::symlink_metadata(&path)?;
            assert!(metadata.is_file());
            assert_eq!(file.size, metadata.len());
            assert_eq!(file.fingerprint, Fingerprint::new(&metadata));

            if pass % backups_per_group == 0 || path == mutable_file_path {
                assert!(file.unique, "{:?} is not unique", path);
            } else {
                assert!(!file.unique, "{:?} is unique", path);
            }

            let data = fs::read(&path)?;
            let hash: Hash = Sha512::digest(&data).as_slice().into();
            assert_eq!(file.hash, hash, "Invalid {:?} hash", path);
        }
    }

    Ok(())
}

fn read_metadata(filesystem: &Filesystem, backup: &Backup) -> GenericResult<HashMap<PathBuf, MetadataItem>> {
    let mut files = HashMap::new();

    for file in backup.read_metadata(filesystem)? {
        let file = file?;
        let path = PathBuf::from(&file.path);
        assert!(files.insert(path, file).is_none());
    }

    assert!(!files.is_empty());
    Ok(files)
}