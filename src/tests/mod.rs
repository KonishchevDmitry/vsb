use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, Permissions};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use assert_fs::fixture::TempDir;
use digest::Digest;
use filetime::FileTime;
use indoc::indoc;
use itertools::Itertools;
use log::info;
use maplit::hashset;
use sha2::Sha512;
use nix::sys::stat::Mode;

use crate::backuping::{self, PathFilter};
use crate::config::{BackupSpecConfig, BackupConfig, BackupItemConfig};
use crate::core::{GenericResult, EmptyResult};
use crate::providers::{ReadProvider, filesystem::Filesystem};
use crate::restoring::Restorer;
use crate::storage::{Backup, Storage};
use crate::storage::metadata::{Fingerprint, MetadataItem};
use crate::util::hash::Hash;

#[test]
fn backup() -> EmptyResult {
    if option_env!("VSB_TESTS_LOGGING") == Some("y") {
        easy_logging::init(module_path!().split("::").next().unwrap(), log::Level::Debug)?;
    }

    let mut git_restorer = GitRestorer::new();

    let sources_path = std::env::current_dir()?;
    let root_path = sources_path.join("src/tests/testdata/root");
    let home_path = root_path.join("home");
    let user_path = home_path.join("user");
    let other_user_path = home_path.join("other-user");
    let var_path = root_path.join("var");

    let mut all_excluded_paths = Vec::new();
    let mut all_excluded_files = Vec::new();

    let skipped_path = var_path.join("skipped");
    all_excluded_paths.push(skipped_path.clone());
    all_excluded_files.push(skipped_path.join("some-file"));
    git_restorer.add(&skipped_path)?;

    let fully_excluded_path = user_path.join("fully-excluded");
    all_excluded_paths.push(fully_excluded_path.clone());
    all_excluded_files.push(fully_excluded_path.join("fully-excluded"));
    git_restorer.add(&fully_excluded_path)?;

    let partially_excluded_path = user_path.join("partially-excluded");
    for name in ["excluded-1", "excluded-2"] {
        let path = partially_excluded_path.join(name);
        all_excluded_paths.push(path.clone());
        all_excluded_files.push(path);
    }
    git_restorer.add(&partially_excluded_path)?;

    let temp_dir = TempDir::new()?;
    let backup_root_path = temp_dir.join("backups");
    fs::create_dir(&backup_root_path)?;

    let max_backup_groups = 2;
    let max_backups_per_group = 5;
    let total_backups = (max_backup_groups + 1) * max_backups_per_group - 1;

    let config = BackupSpecConfig {
        name: "test".to_owned(),
        path: backup_root_path.to_str().unwrap().to_owned(),
        backup: Some(BackupConfig {
            items: vec![BackupItemConfig {
                path: root_path.join("etc").to_str().unwrap().to_owned(),
                filter: PathFilter::default(),
            }, BackupItemConfig {
                path: user_path.to_str().unwrap().to_owned(),
                filter: PathFilter::new(indoc!("
                - fully-excluded
                + partially-excluded/included-*
                - partially-excluded/*
            "))?,
            }, BackupItemConfig {
                path: other_user_path.to_str().unwrap().to_owned(),
                filter: PathFilter::default(),
            }, BackupItemConfig {
                path: var_path.join("data").to_str().unwrap().to_owned(),
                filter: PathFilter::default(),
            }],
            max_backup_groups,
            max_backups_per_group,
        }),
        upload: None
    };

    // Check permissions preserving for backup root parent directories
    fs::set_permissions(&home_path, Permissions::from_mode(0o704))?;

    // Check permissions preserving for directories
    let permissions_dir_path = user_path.join("permissions");
    fs::set_permissions(&permissions_dir_path, Permissions::from_mode((
        Mode::from_bits(0o511).unwrap() | Mode::S_ISUID | Mode::S_ISGID | Mode::S_ISVTX
    ).bits().into()))?;

    // Check permissions preserving for files
    let permissions_file_path = permissions_dir_path.join("permissions");
    fs::set_permissions(&permissions_file_path, Permissions::from_mode((
        Mode::from_bits(0o404).unwrap() | Mode::S_ISUID | Mode::S_ISGID | Mode::S_ISVTX
    ).bits().into()))?;

    let mut mutable_files_states = Vec::new();
    let mutable_file_path = user_path.join("mutable");
    let same_mutable_orig_file_path = user_path.join("same-mutable-1/nested/same-mutable");
    let same_mutable_extern_file_path = user_path.join("same-mutable-2/nested/same-mutable");
    let periodically_mutable_file_path = user_path.join("periodically-mutable");
    let periodically_existing_file_path = user_path.join("periodically-existing");
    let periodically_same_existing_file_path = user_path.join("periodically-same-existing");

    // Restoring logic will have to create extern file's directories before it'll see them in the
    // archive.
    for path in [&same_mutable_orig_file_path, &same_mutable_extern_file_path] {
        let nested = path.parent().unwrap();
        let parent = nested.parent().unwrap();

        for dir in [parent, nested] {
            match fs::create_dir(dir) {
                Ok(()) => {},
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {},
                Err(e) => return Err!("Failed to create {:?}: {}", dir, e),
            }
        }
    }

    let storage = Storage::new_read_only(Filesystem::new(), backup_root_path.to_str().unwrap());

    for pass in 0..total_backups {
        info!("Backup #{} pass...", pass);

        mutable_files_states.push(vec![
            FileState::new(&mutable_file_path, Some(format!("pass-{}", pass)))?,
            FileState::new(&same_mutable_orig_file_path, Some(format!("same-pass-{}", pass)))?,
            FileState::new(&same_mutable_extern_file_path, Some(format!("same-pass-{}", pass)))?,
            FileState::new(&periodically_mutable_file_path, Some(format!("periodically-{}", pass / 2 * 2)))?,
            FileState::new(&periodically_existing_file_path, if pass % 2 == 0 {
                Some(format!("periodically-existing-{}", pass))
            } else {
                None
            })?,
            FileState::new(&periodically_same_existing_file_path, if pass % 2 != 0 {
                Some("Periodically same existing file".to_owned())
            } else {
                None
            })?,
        ]);

        assert!(backuping::backup(&config)?);

        let (groups, ok) = storage.get_backup_groups(true)?;
        assert!(ok);
        assert!(groups.iter().all(|group| group.temporary_backups.is_empty()));
        assert_eq!(groups.len(), std::cmp::min(pass / max_backups_per_group + 1, max_backup_groups));

        let group = groups.last().unwrap();
        assert_eq!(group.backups.len(), pass % max_backups_per_group + 1);

        let backup = group.backups.last().unwrap();
        let files = read_metadata(storage.provider.read(), backup)?;

        for path in &all_excluded_files {
            assert!(path.exists(), "{:?} doesn't exist", path);
            assert!(!files.contains_key(path), "Metadata contains {:?}", path);
        }
        assert!(!files.contains_key(&user_path));
        assert!(files.contains_key(&mutable_file_path));

        let always_unique = hashset! {
            &mutable_file_path,
            &same_mutable_orig_file_path,
            &periodically_existing_file_path,
        };

        let always_extern = hashset! {
            user_path.join("empty"),
            user_path.join("other-empty"),
            user_path.join("same-contents-2"),
            same_mutable_extern_file_path.clone(),
        };

        for (path, file) in files {
            let metadata = fs::symlink_metadata(&path)?;
            assert!(metadata.is_file());
            assert_eq!(file.size, metadata.len());
            assert_eq!(file.fingerprint, Fingerprint::new(&metadata));

            let expected_unique =
                pass % max_backups_per_group == 0 && !always_extern.contains(&path) ||
                path == periodically_mutable_file_path && pass % 2 == 0 ||
                path == periodically_same_existing_file_path && [1, 5, 11].contains(&pass) ||
                always_unique.contains(&path);
            assert_eq!(file.unique, expected_unique, "{}: unique={}", path.display(), file.unique);

            let data = fs::read(&path)?;
            let hash: Hash = Sha512::digest(&data).as_slice().into();
            assert_eq!(file.hash, hash, "Invalid {:?} hash", path);
        }
    }

    let var_time = fs::metadata(&var_path)?.modified()?;
    let partially_excluded_time = fs::metadata(&partially_excluded_path)?.modified()?;

    for path in &all_excluded_paths {
        if fs::symlink_metadata(path)?.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }

    filetime::set_file_mtime(&var_path, FileTime::from_system_time(var_time))?;
    filetime::set_file_mtime(&partially_excluded_path, FileTime::from_system_time(partially_excluded_time))?;

    let (groups, ok) = storage.get_backup_groups(true)?;
    assert!(ok);
    assert!(groups.iter().all(|group| group.temporary_backups.is_empty()));

    info!("Restoring the following groups:");
    for group in &groups {
        info!("* {}: {}", group.name, group.backups.iter().map(|backup| &backup.name).join(", "));
    }

    let mut restore_pass = max_backups_per_group; // First group has been deleted as old

    for group in groups {
        for backup in group.backups {
            info!("Restore #{} pass ({})...", restore_pass, backup.name);
            let restore_dir = temp_dir.join("restore");

            let restorer = Restorer::new(Path::new(&backup.path))?;
            assert!(restorer.restore(&restore_dir)?);

            for file_state in &mutable_files_states[restore_pass] {
                file_state.restore()?;
            }

            let restored_root_path = get_restore_path(&restore_dir, &root_path);

            let ls_tree_command = "ls -ARl --time-style +%Y.%m.%d-%H:%M:%S";
            shell(&format!(
                "set -o pipefail && diff -u <(cd {:?} && {}) <(cd {:?} && {})",
                root_path, ls_tree_command, restored_root_path, ls_tree_command
            ))?;

            run(["git", "diff", "--no-index", root_path.to_str().unwrap(), restored_root_path.to_str().unwrap()])?;

            fs::set_permissions(get_restore_path(&restore_dir, &permissions_dir_path), Permissions::from_mode(0o700))?;
            fs::remove_dir_all(restore_dir)?;
            restore_pass += 1;
        }
    }

    assert_eq!(restore_pass, total_backups);
    temp_dir.close()?;

    Ok(())
}

struct GitRestorer(Vec<PathBuf>);

impl GitRestorer {
    fn new() -> GitRestorer {
        GitRestorer(Vec::new())
    }

    fn add<P: AsRef<Path>>(&mut self, path: P) -> EmptyResult {
        let path = path.as_ref();
        GitRestorer::restore(path)?;
        self.0.push(path.to_owned());
        Ok(())
    }

    fn restore(path: &Path) -> EmptyResult {
        run(["git", "checkout", "--quiet", path.to_str().unwrap()])
    }
}

impl Drop for GitRestorer {
    fn drop(&mut self) {
        for path in self.0.drain(..) {
            GitRestorer::restore(&path).unwrap();
        }
    }
}

struct FileState {
    path: PathBuf,
    contents: Option<(String, SystemTime)>,

    parent_path: PathBuf,
    parent_modify_time: SystemTime,
}

impl FileState {
    fn new(path: &Path, contents: Option<String>) -> GenericResult<FileState> {
        let parent_path = path.parent().ok_or_else(|| format!("Invalid file path: {:?}", path))?;

        let contents = if let Some(contents) = contents {
            fs::write(&path, &contents)?;
            Some((contents, fs::metadata(path)?.modified()?))
        } else {
            if let Err(err) = fs::remove_file(path) {
                if err.kind() != ErrorKind::NotFound {
                    return Err(err.into());
                }
            }
            None
        };

        Ok(FileState {
            path: path.to_owned(), contents,
            parent_path: parent_path.to_owned(),
            parent_modify_time: fs::metadata(parent_path)?.modified()?,
        })
    }

    fn restore(&self) -> EmptyResult {
        if let Some((ref contents, modify_time)) = self.contents {
            fs::write(&self.path, contents)?;
            filetime::set_file_mtime(&self.path, FileTime::from_system_time(modify_time))?;
        } else {
            if let Err(err) = fs::remove_file(&self.path) {
                if err.kind() != ErrorKind::NotFound {
                    return Err(err.into());
                }
            }
        };

        filetime::set_file_mtime(&self.parent_path, FileTime::from_system_time(self.parent_modify_time))?;
        Ok(())
    }
}

fn run<C: IntoIterator<Item=A>, A: AsRef<OsStr>>(command: C) -> EmptyResult {
    let mut command = command.into_iter();

    let status = Command::new(command.next().unwrap()).args(command).status()?;
    if !status.success() {
        return Err!("{}", status);
    }

    Ok(())
}

fn shell(command: &str) -> EmptyResult {
    run(["bash", "-ec", command])
}

fn get_restore_path(restore_dir: &Path, path: &Path) -> PathBuf {
    let mut components = path.components();
    assert_eq!(components.next(), Some(Component::RootDir));
    restore_dir.join(components.as_path())
}

fn read_metadata(provider: &dyn ReadProvider, backup: &Backup) -> GenericResult<HashMap<PathBuf, MetadataItem>> {
    let mut files = HashMap::new();

    for file in backup.read_metadata(provider)? {
        let file = file?;
        let path = PathBuf::from(&file.path);
        assert!(files.insert(path, file).is_none());
    }

    assert!(!files.is_empty());
    Ok(files)
}