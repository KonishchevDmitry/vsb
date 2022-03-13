// FIXME(konishchev): Handle file truncation during backing up properly

mod multi_writer;

use std::collections::{HashMap, HashSet, hash_map::Entry as HashMapEntry};
use std::fs::{self, OpenOptions};
use std::io;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf, Component};

use itertools::Itertools;
use log::{error, info};
use serde_json::Value;
use tar::{Archive, Header, Entry, EntryType};

use crate::core::{EmptyResult, GenericError, GenericResult};
use crate::file_reader::FileReader;
use crate::hash::Hash;
use crate::providers::filesystem::Filesystem;
use crate::storage::{Storage, StorageRc, BackupGroup, Backup, BackupTraits};

use multi_writer::MultiWriter;

pub struct Restorer {
    storage: StorageRc,
    group_name: String,
    backup_name: String,
    directories: HashSet<PathBuf>, // FIXME(konishchev): Check on finalization
    permissions: Vec<(PathBuf, Permissions)>,
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
            directories: HashSet::new(),
            permissions: Vec::new(),
        })
    }

    // FIXME(konishchev): Implement
    pub fn restore(mut self, restore_path: &Path) -> GenericResult<bool> {
        let mut ok = true;
        let plan = RestorePlan::new(&self.storage, &self.group_name, &self.backup_name)?;

        // FIXME(konishchev): Permissions
        fs::create_dir(restore_path).map_err(|e| format!(
            "Failed to create {:?}: {}", restore_path, e))?;

        for (index, step) in plan.steps.iter().enumerate() {
            ok &= self.process_step(step, index == 0, restore_path).map_err(|e| format!(
                "Failed to restore {:?} backup: {}", step.backup.path, e))?;
        }

        Ok(ok)
    }

    fn process_step(&mut self, step: &RestoreStep, is_target: bool, restore_dir: &Path) -> GenericResult<bool> {
        let mut ok = true;
        let mut archive = step.backup.read_data(self.storage.provider.read())?;

        for entry in archive.entries()? {
            let mut entry = entry?;
            let header = entry.header();
            let entry_path = entry.path()?;
            let entry_type = header.entry_type();
            let file_path = get_file_path(&entry_path)?;

            // FIXME(konishchev): HERE
            match entry_type {
                EntryType::Directory => if is_target {
                    let restore_path = get_restore_path(restore_dir, &file_path)?;

                    if !self.directories.remove(&file_path) {
                        // FIXME(konishchev): Permissions
                        fs::create_dir(&restore_path).map_err(|e| format!(
                            "Unable to create {:?}: {}", restore_path, e))?;
                    }

                    self.schedule_permissions(restore_path, header)?;
                }

                EntryType::Regular => {
                    if let Some(info) = step.files.get(&file_path) {
                        self.restore_files(&file_path, &mut entry, info, restore_dir, is_target)?;
                    } else if is_target {
                        // FIXME(konishchev): Ensure external
                        if entry.size() != 0 {
                            error!("The backup archive has data for {:?} file which is expected to be external.", file_path);
                            ok = false;
                        }
                    }
                },

                EntryType::Symlink => {
                    let target = entry.link_name()
                        .map_err(|e| format!("Got an invalid {:?} symlink target path: {}", file_path, e))?
                        .ok_or_else(|| format!("Got {:?} symlink without target path", file_path))?;

                    let restore_path = get_restore_path(restore_dir, file_path)?;
                    std::os::unix::fs::symlink(target, &restore_path).map_err(|e| format!(
                        "Unable to create {:?} symlink: {}", restore_path, e))?;
                },

                _ => {
                    // FIXME(konishchev): Support
                    return Err!("Got an unsupported entry ({:?}): {:?}", entry_type, entry_path)
                }
            }
        }

        Ok(ok)
    }

    // FIXME(konishchev): Permissions
    fn restore_files(
        &mut self, source_path: &Path, data: &mut dyn Read, info: &FileInfo,
        restore_dir: &Path, is_target: bool,
    ) -> EmptyResult {
        let paths = info.paths.iter().map(|path| format!("{:?}", path)).join(", ");
        info!("Restoring {}...", paths);

        let mut files = Vec::new();

        for path in &info.paths {
            let restore_path = get_restore_path(restore_dir, path)?;

            if is_target && path != source_path {
                self.directories.extend(restore_dirs(restore_dir, path)?);
            }

            files.push(OpenOptions::new()
                .write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW)
                .open(&restore_path).map_err(|e| format!("Unable to create {:?}: {}", restore_path, e))?);
        }

        let mut files = MultiWriter::new(files);
        let mut reader = FileReader::new(data, info.size);

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

        Ok(())
    }

    // FIXME(konishchev): Support
    fn schedule_permissions(&mut self, path: PathBuf, header: &Header) -> EmptyResult {
        let permissions = self.get_permissions(header)?;
        self.permissions.push((path.to_path_buf(), permissions));
        Ok(())
    }

    // FIXME(konishchev): Implement
    fn get_permissions(&self, header: &Header) -> GenericResult<Permissions> {
        Ok(Permissions {})
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

                    if file.unique {
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

struct Permissions {
}

// FIXME(konishchev): Drop
struct RestoreStepOld {
    backup: Backup,
    files: HashMap<String, Vec<String>>,
}

// FIXME(konishchev): Implement
fn unpack_in<R: Read>(entry: Entry<R>, dst: &Path) -> EmptyResult {

/*
        if self.preserve_mtime {
            if let Ok(mtime) = self.header.mtime() {
                // For some more information on this see the comments in
                // `Header::fill_platform_from`, but the general idea is that
                // we're trying to avoid 0-mtime files coming out of archives
                // since some tools don't ingest them well. Perhaps one day
                // when Cargo stops working with 0-mtime archives we can remove
                // this.
                let mtime = if mtime == 0 { 1 } else { mtime };
                let mtime = FileTime::from_unix_time(mtime as i64, 0);
                filetime::set_file_handle_times(&f, Some(mtime), Some(mtime)).map_err(|e| {
                    TarError::new(format!("failed to set mtime for `{}`", dst.display()), e)
                })?;
            }
        }
        if let Ok(mode) = self.header.mode() {
            set_perms(dst, Some(&mut f), mode, self.preserve_permissions)?;
        }

        fn set_perms(
            dst: &Path,
            f: Option<&mut std::fs::File>,
            mode: u32,
            preserve: bool,
        ) -> Result<(), TarError> {
            _set_perms(dst, f, mode, preserve).map_err(|e| {
                TarError::new(
                    format!(
                        "failed to set permissions to {:o} \
                         for `{}`",
                        mode,
                        dst.display()
                    ),
                    e,
                )
            })
        }

        #[cfg(unix)]
        fn _set_perms(
            dst: &Path,
            f: Option<&mut std::fs::File>,
            mode: u32,
            preserve: bool,
        ) -> io::Result<()> {
            use std::os::unix::prelude::*;

            let mode = if preserve { mode } else { mode & 0o777 };
            let perm = fs::Permissions::from_mode(mode as _);
            match f {
                Some(f) => f.set_permissions(perm),
                None => fs::set_permissions(dst, perm),
            }
        }
 */

    Ok(())
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

fn restore_dirs<R, P>(restore_dir: R, file_path: P) -> GenericResult<Vec<PathBuf>>
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

// FIXME(konishchev): User/group
// FIXME(konishchev): Implement
/*
fn set_perms(
    dst: &Path,
    f: Option<&mut std::fs::File>,
    mode: u32,
    preserve: bool,
) -> io::Result<()> {
    use std::os::unix::prelude::*;

    let mode = if preserve { mode } else { mode & 0o777 };
    let perm = fs::Permissions::from_mode(mode as _);
    match f {
        Some(f) => f.set_permissions(perm),
        None => fs::set_permissions(dst, perm),
    }
}
 */