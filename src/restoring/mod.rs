// FIXME(konishchev): Handle file truncation during backing up properly

mod file_metadata;
mod multi_writer;

use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io;
use std::io::{ErrorKind, Read};
use std::os::unix::{self, fs::{DirBuilderExt, OpenOptionsExt}};
use std::path::{Path, PathBuf, Component};

use itertools::Itertools;
use log::{error, info};
use tar::{Entry, EntryType, Header};

use crate::core::{EmptyResult, GenericError, GenericResult};
use crate::file_reader::FileReader;
use crate::hash::Hash;
use crate::providers::filesystem::Filesystem;
use crate::storage::{Storage, StorageRc, Backup};
use crate::users::UsersCache;

use file_metadata::{FileMetadata, Owner};
use multi_writer::MultiWriter;

pub struct Restorer {
    storage: StorageRc,
    group_name: String,
    backup_name: String,

    users: Option<UsersCache>,
    directories: HashSet<PathBuf>, // FIXME(konishchev): Check on finalization
    file_metadata: Vec<(PathBuf, FileMetadata)>,
}

impl Restorer {
    pub fn new(backup_path: &Path) -> GenericResult<Restorer> {
        let backup_path = backup_path.canonicalize().map_err(|e| format!(
            "Invalid backup path: {}", e))?;

        let (backup_root, group_name, backup_name) = {
            let backup_name = backup_path.file_name().and_then(|name| name.to_str());
            let group_path = backup_path.parent();
            let group_name = group_path.and_then(|path| path.file_name()).and_then(|name| name.to_str());
            let backup_root = group_path.and_then(|path| path.parent()).and_then(|name| name.to_str());

            match (backup_root, group_name, backup_name) {
                (Some(root), Some(group_name), Some(backup_name)) => (root, group_name, backup_name),
                _ => return Err!("Invalid backup path"),
            }
        };

        let storage = Storage::new(Filesystem::new(), backup_root);
        let backup_traits = storage.backup_traits();

        if
            !backup_traits.group_name_regex.is_match(group_name) ||
            !backup_traits.name_regex.is_match(backup_name)
        {
            return Err!("{:?} doesn't look like backup path", backup_path)
        }

        Ok(Restorer {
            storage,
            group_name: group_name.to_owned(),
            backup_name: backup_name.to_owned(),

            users: if nix::unistd::geteuid().is_root() {
                Some(UsersCache::new())
            } else {
                None
            },

            directories: HashSet::new(),
            file_metadata: Vec::new(),
        })
    }

    // FIXME(konishchev): Implement
    pub fn restore(mut self, restore_dir: &Path) -> GenericResult<bool> {
        let mut ok = true;
        let plan = RestorePlan::new(&self.storage, &self.group_name, &self.backup_name)?;

        // FIXME(konishchev): Permissions
        fs::create_dir(restore_dir).map_err(|e| format!(
            "Failed to create {:?}: {}", restore_dir, e))?;

        for (index, step) in plan.steps.iter().enumerate() {
            ok &= self.process_step(step, index == 0, restore_dir).map_err(|e| format!(
                "Failed to restore {:?} backup: {}", step.backup.path, e))?;
        }

        for (path, metadata) in self.file_metadata {
            // FIXME(konishchev): Don't fail, add checks
            metadata.set(get_restore_path(restore_dir, path)?)?;
        }

        if !self.directories.is_empty() {
            error!("The following directories have been unexpectedly created without known permissions:");
            for path in self.directories {
                error!("* {}", path.display());
            }
            ok = false;
        }

        Ok(ok)
    }

    fn process_step(&mut self, step: &RestoreStep, is_target: bool, restore_dir: &Path) -> GenericResult<bool> {
        let mut ok = true;
        let mut archive = step.backup.read_data(self.storage.provider.read())?;

        for entry in archive.entries()? {
            let entry = entry?;
            let header = entry.header();
            let entry_path = entry.path()?;
            let entry_type = header.entry_type();
            let file_path = get_file_path(&entry_path)?;

            match entry_type {
                EntryType::Directory => if is_target {
                    if !self.directories.remove(&file_path) {
                        create_directory(get_restore_path(restore_dir, &file_path)?)?;
                    }
                    self.schedule_file_metadata_change(file_path, header)?;
                }

                EntryType::Regular => {
                    if let Some(info) = step.files.get(&file_path) {
                        self.restore_files(&file_path, entry, info, restore_dir, is_target)?;
                    } else if is_target {
                        // FIXME(konishchev): Ensure external
                        if entry.size() != 0 {
                            error!("The backup archive has data for {:?} file which is expected to be external.", file_path);
                            ok = false;
                        }
                        self.schedule_file_metadata_change(file_path, header)?;
                    }
                },

                EntryType::Symlink => if is_target {
                    let target = entry.link_name()
                        .map_err(|e| format!("Got an invalid {:?} symlink target path: {}", file_path, e))?
                        .ok_or_else(|| format!("Got {:?} symlink without target path", file_path))?;

                    let restore_path = get_restore_path(restore_dir, file_path)?;
                    unix::fs::symlink(target, &restore_path).map_err(|e| format!(
                        "Unable to create {:?} symlink: {}", restore_path, e))?;

                    self.get_file_metadata(header)?.set(&restore_path)?;
                },

                _ => {
                    return Err!(
                        "Got an unsupported archive entry ({:?}): {:?}",
                        entry_type, entry_path)
                }
            }
        }

        Ok(ok)
    }

    // FIXME(konishchev): Permissions
    fn restore_files(
        &mut self, source_path: &Path, mut entry: Entry<Box<dyn Read>>, info: &FileInfo,
        restore_dir: &Path, is_target: bool,
    ) -> EmptyResult {
        let paths = info.paths.iter().map(|path| format!("{:?}", path)).join(", ");
        info!("Restoring {}...", paths);

        let mut files = Vec::new();
        let mut restore_metadata = None;

        for path in &info.paths {
            let restore_path = get_restore_path(restore_dir, path)?;

            if is_target {
                if path == source_path {
                    let metadata = self.get_file_metadata(entry.header())?;
                    assert!(restore_metadata.replace((restore_path.clone(), metadata)).is_none());
                } else {
                    // FIXME(konishchev): Check also that we'll change all file permissions?
                    self.directories.extend(restore_directories(restore_dir, path)?);
                }
            }

            files.push(OpenOptions::new()
                .write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW)
                .open(&restore_path).map_err(|e| format!("Unable to create {:?}: {}", restore_path, e))?);
        }

        let mut files = MultiWriter::new(files);
        let mut reader = FileReader::new(&mut entry, info.size);

        io::copy(&mut reader, &mut files).map_err(|e| format!(
            "Failed to restore {}: {}", paths, e))?;
        let (bytes_read, hash) = reader.consume();

        if bytes_read != info.size {
            return Err!(
                "Failed to restore {}: got an unexpected data size: {} vs {}",
                paths, bytes_read, info.size);
        }

        if hash != info.hash {
            return Err!(
                "Failed to restore {}: the restored data has an unexpected hash: {} vs {}",
                paths, hash, info.hash);
        }

        if let Some((path, metadata)) = restore_metadata {
            metadata.set(&path)?;
        }

        Ok(())
    }

    // FIXME(konishchev): Support
    fn schedule_file_metadata_change(&mut self, path: PathBuf, header: &Header) -> EmptyResult {
        self.file_metadata.push((path, self.get_file_metadata(header)?));
        Ok(())
    }

    fn get_file_metadata(&self, header: &Header) -> GenericResult<FileMetadata> {
        fn map_err<E: Display>(header: &Header, name: &str, err: E) -> String {
            format!("Got an invalid {}{} from archive: {}", name, match header.path() {
                Ok(path) => format!(" for {:?}", path),
                Err(_) => String::new(),
            }, err)
        }

        let owner = self.users.as_ref().map(|users| -> GenericResult<Owner> {
            let mut uid = header.uid()?.try_into().map_err(|e| map_err(header, "user ID", e))?;
            if let Some(name) = header.username().map_err(|e| map_err(header, "user name", e))? {
                if let Some(id) = users.get_uid(name)? {
                    uid = id;
                }
            }

            let mut gid = header.gid()?.try_into().map_err(|e| map_err(header, "group ID", e))?;
            if let Some(name) = header.groupname().map_err(|e| map_err(header, "group name", e))? {
                if let Some(id) = users.get_gid(name)? {
                    gid = id;
                }
            }

            Ok(Owner {uid, gid})
        }).transpose()?;

        let mode = if header.entry_type() == EntryType::Symlink {
            None
        } else {
            Some(header.mode()?)
        };

        let mtime = header.mtime()?.try_into().map_err(|e| map_err(
            header, "file modification time", e))?;

        Ok(FileMetadata {owner, mode, mtime})
    }
}

struct FileInfo {
    hash: Hash,
    size: u64,
    paths: Vec<PathBuf>,
}

struct RestoreStep {
    backup: Backup,
    files: HashMap<PathBuf, FileInfo>,
}

struct RestorePlan {
    steps: Vec<RestoreStep>,
    missing: HashSet<PathBuf>,
}

impl RestorePlan {
    fn new(storage: &Storage, group_name: &str, backup_name: &str) -> GenericResult<RestorePlan> {
        let provider = storage.provider.read();
        let group = storage.get_backup_group(group_name, true)?;

        let mut steps = Vec::new();
        let mut extern_files: HashMap<Hash, Vec<PathBuf>> = HashMap::new();

        for backup in group.backups.into_iter().rev() {
            if steps.is_empty() && backup.name != backup_name {
                continue;
            }

            let map_read_error = |error: GenericError| -> String {
                return format!("Error while reading {:?} backup metadata: {}", backup.path, error);
            };

            let mut to_restore = HashMap::new();

            if steps.is_empty() {
                let mut unique_files = Vec::new();

                for file in backup.read_metadata(provider).map_err(map_read_error)? {
                    let file = file.map_err(map_read_error)?;
                    let path = PathBuf::from(file.path);

                    if file.unique || file.size == 0 {
                        unique_files.push((path, file.hash, file.size));
                    } else {
                        extern_files.entry(file.hash).or_default().push(path);
                    }
                }

                for (path, hash, size) in unique_files {
                    let mut paths = extern_files.remove(&hash).unwrap_or_default();
                    paths.reserve_exact(1);
                    paths.push(path.clone());
                    to_restore.insert(path, FileInfo {hash, size, paths});
                }
            } else {
                if extern_files.is_empty() {
                    break;
                }

                for file in backup.read_metadata(provider).map_err(map_read_error)? {
                    let file = file.map_err(map_read_error)?;
                    if let Some(paths) = extern_files.remove(&file.hash) {
                        to_restore.insert(file.path.into(), FileInfo {
                            hash: file.hash,
                            size: file.size,
                            paths
                        });
                    }
                }
            }

            if steps.is_empty() || !to_restore.is_empty() {
                steps.push(RestoreStep {backup, files: to_restore});
            }
        }

        if steps.is_empty() {
            return Err!("The backup doesn't exist");
        }

        let mut missing = HashSet::new();
        for paths in extern_files.into_values() {
            for path in paths {
                missing.insert(path);
            }
        }

        Ok(RestorePlan {steps, missing})
    }
}

fn get_file_path<P: AsRef<Path>>(file_path: P) -> GenericResult<PathBuf> {
    let file_path = file_path.as_ref();
    let mut path = PathBuf::from("/");

    let mut changed = false;

    for part in file_path.components() {
        if let Component::Normal(part) = part {
            path.push(part);
            changed = true;
        } else {
            return Err!("Got an invalid file path from archive: {:?}", file_path);
        }
    }

    if !changed {
        return Err!("Got an invalid file path from archive: {:?}", file_path);
    }

    Ok(path)
}

fn get_restore_path<R, P>(restore_dir: R, file_path: P) -> GenericResult<PathBuf>
    where R: AsRef<Path>, P: AsRef<Path>
{
    let file_path = file_path.as_ref();
    let mut restore_path = restore_dir.as_ref().to_path_buf();

    let mut changed = false;

    for (index, part) in file_path.components().enumerate() {
        match part {
            Component::RootDir if index == 0 => {},

            Component::Normal(part) if index != 0 => {
                restore_path.push(part);
                changed = true;
            },

            _ => return Err!("Invalid restoring file path: {:?}", file_path),
        }
    }

    if !changed {
        return Err!("Invalid restoring file path: {:?}", file_path);
    }

    Ok(restore_path)
}

fn restore_directories<R, P>(restore_dir: R, file_path: P) -> GenericResult<Vec<PathBuf>>
    where R: AsRef<Path>, P: AsRef<Path>
{
    let mut path = file_path.as_ref();
    let mut to_restore = Vec::new();
    let mut restored_dirs = Vec::new();

    loop {
        path = path.parent().ok_or_else(|| format!(
            "Invalid restoring file path: {:?}", file_path.as_ref()))?;

        if path == Path::new("/") {
            break;
        }

        let restore_path = get_restore_path(&restore_dir, path)?;

        // FIXME(konishchev): Permissions
        match fs::create_dir(&restore_path) {
            Ok(_) => {
                restored_dirs.push(path.to_owned());
                break;
            },
            Err(err) => match err.kind() {
                ErrorKind::NotFound => {
                    to_restore.push(path.to_owned());
                },
                ErrorKind::AlreadyExists => {
                    break;
                }
                _ => return Err!("Unable to create {:?}: {}", restore_path, err),
            }
        }
    }

    for path in to_restore {
        let restore_path = get_restore_path(&restore_dir, &path)?;
        // FIXME(konishchev): Permissions
        fs::create_dir(&restore_path).map_err(|e| format!(
            "Unable to create {:?}: {}", restore_path, e))?;
        restored_dirs.push(path);
    }

    Ok(restored_dirs)
}

fn create_directory<P: AsRef<Path>>(path: P) -> EmptyResult {
    let path = path.as_ref();
    Ok(DirBuilder::new().mode(0o700).create(path).map_err(|e| format!(
        "Unable to create {:?}: {}", path, e))?)
}