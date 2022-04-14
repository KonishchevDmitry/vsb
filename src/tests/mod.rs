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
use log::info;
use maplit::hashset;
use sha2::Sha512;
use nix::sys::stat::Mode;

use crate::backuping::{Backuper, BackupInstance};
use crate::config::{BackupConfig, BackupItemConfig};
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

    let root_dir_name = PathBuf::from("src/tests/testdata/root");
    let skipped_dir_name = root_dir_name.join("skipped");
    let _git_restorer = RestoreGitFiles::new(&skipped_dir_name);

    let temp_dir = TempDir::new()?;
    let backup_root_path = temp_dir.join("backups");
    fs::create_dir(&backup_root_path)?;

    let sources_path = std::env::current_dir()?;
    let root_path = sources_path.join(root_dir_name);
    let home_path = root_path.join("home");
    let user_path = home_path.join("user");
    let other_user_path = home_path.join("other-user");
    let skipped_path = sources_path.join(skipped_dir_name);

    let total_backups = 9;
    let backups_per_group = 5;

    let backup_config = BackupConfig {
        name: s!("test"),
        path: backup_root_path.to_str().unwrap().to_owned(),
        max_backups: backups_per_group,

        items: Some(vec![BackupItemConfig {
            path: user_path.to_str().unwrap().to_owned(),
        }, BackupItemConfig {
            path: other_user_path.to_str().unwrap().to_owned(),
        }, BackupItemConfig {
            path: root_path.join("etc").to_str().unwrap().to_owned(),
        }]),
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

    let storage = Storage::new(Filesystem::new(), backup_root_path.to_str().unwrap());

    for pass in 0..total_backups {
        info!("Backup #{} pass...", pass);

        mutable_files_states.push(vec![
            FileState::new(&mutable_file_path, format!("pass-{}", pass))?,
            FileState::new(&same_mutable_orig_file_path, format!("same-pass-{}", pass))?,
            FileState::new(&same_mutable_extern_file_path, format!("same-pass-{}", pass))?,
            FileState::new(&periodically_mutable_file_path, format!("periodically-{}", pass / 2 * 2))?,
        ]);

        let (backup, ok) = BackupInstance::create(&backup_config, storage.clone())?;
        assert!(ok);

        let backuper = Backuper::new(&backup_config, backup)?;
        assert!(backuper.run()?);

        let (groups, ok) = storage.get_backup_groups(true)?;
        assert!(ok);
        assert_eq!(groups.len(), pass / backups_per_group + 1);

        let group = groups.last().unwrap();
        assert_eq!(group.backups.len(), pass % backups_per_group + 1);

        let backup = group.backups.last().unwrap();
        let files = read_metadata(storage.provider.read(), backup)?;

        // FIXME(konishchev): Add filtered files here
        for file in [&skipped_path] {
            assert!(file.exists());
            assert!(!files.contains_key(file));
        }
        assert!(!files.contains_key(&user_path));
        assert!(files.contains_key(&mutable_file_path));

        let always_unique = hashset! {
            &mutable_file_path,
            &same_mutable_orig_file_path,
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
                pass % backups_per_group == 0 && !always_extern.contains(&path) ||
                path == periodically_mutable_file_path && pass % 2 == 0 ||
                always_unique.contains(&path);
            assert_eq!(file.unique, expected_unique, "{}: unique={}", path.display(), file.unique);

            let data = fs::read(&path)?;
            let hash: Hash = Sha512::digest(&data).as_slice().into();
            assert_eq!(file.hash, hash, "Invalid {:?} hash", path);
        }
    }

    let (groups, ok) = storage.get_backup_groups(true)?;
    assert!(ok);

    let mut restore_pass = 0;
    fs::remove_dir_all(skipped_path)?;

    for group in groups {
        for backup in group.backups {
            info!("Restore #{} pass...", restore_pass);
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

struct RestoreGitFiles(PathBuf);

impl RestoreGitFiles {
    fn new<P: AsRef<Path>>(path: P) -> GenericResult<RestoreGitFiles> {
        let mut restorer = RestoreGitFiles(path.as_ref().to_owned());
        restorer.restore()?;
        Ok(restorer)
    }

    fn restore(&mut self) -> EmptyResult {
        run(["git", "checkout", "--quiet", self.0.to_str().unwrap()])
    }
}

impl Drop for RestoreGitFiles {
    fn drop(&mut self) {
        self.restore().unwrap();
    }
}

struct FileState {
    path: PathBuf,
    contents: String,
    modify_time: SystemTime,
}

impl FileState {
    fn new(path: &Path, contents: String) -> GenericResult<FileState> {
        fs::write(&path, &contents)?;

        Ok(FileState {
            path: path.to_owned(),
            contents,
            modify_time: fs::metadata(path)?.modified()?,
        })
    }

    fn restore(&self) -> EmptyResult {
        fs::write(&self.path, &self.contents)?;
        filetime::set_file_mtime(&self.path, FileTime::from_system_time(self.modify_time))?;
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