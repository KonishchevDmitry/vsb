use std::collections::HashSet;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, FileTypeExt};
use std::path::{Component, Path, PathBuf};

use itertools::Itertools;
use log::{debug, warn, error};
use nix::fcntl::OFlag;

use crate::config::{BackupConfig, BackupItemConfig};
use crate::core::{EmptyResult, GenericError, GenericResult};
use crate::util;

use super::BackupInstance;

pub struct Backuper<'a> {
    items: &'a Vec<BackupItemConfig>,
    backup: BackupInstance,

    roots: Vec<PathBuf>,
    root_parents: HashSet<PathBuf>,
    ok: bool,
}

impl<'a> Backuper<'a> {
    pub fn new(config: &BackupConfig, backup: BackupInstance) -> GenericResult<Backuper> {
        let items = config.items.as_ref().ok_or(
            "Backup items aren't configured for the specified backup")?;

        Ok(Backuper {
            items, backup,
            roots: Vec::new(),
            root_parents: HashSet::new(),
            ok: true,
        })
    }

    pub fn run(mut self) -> GenericResult<bool> {
        for item in self.items {
            match self.prepare(&item) {
                Ok(path) => self.backup_path(&path, true)?,
                Err(err) => self.handle_path_error(Path::new(&item.path), err)?,
            }
        }

        self.backup.finish()?;
        Ok(self.ok)
    }

    fn prepare(&mut self, item: &BackupItemConfig) -> GenericResult<PathBuf> {
        let path = item.path()?;

        for backup_root in &self.roots {
            if path.starts_with(backup_root) || backup_root.starts_with(&path) {
                return Err!("it intersects with previously backed up path");
            }
        }

        self.roots.push(path.clone());
        Ok(path)
    }

    fn backup_path(&mut self, path: &Path, top_level: bool) -> EmptyResult {
        debug!("Backing up {:?}...", path);

        if let Err(err) = crate::metadata::validate_path(path) {
            return self.handle_path_error(path, err);
        }

        if top_level {
            if !self.backup_parent_directories(path)? {
                return Ok(());
            }
        }

        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) => {
                return self.handle_access_error(path, top_level, err, None);
            },
        };

        let file_type = metadata.file_type();

        if file_type.is_file() {
            self.backup_file(path, top_level)?;
        } else if file_type.is_dir() {
            self.backup_directory(path, top_level, metadata)?;
        } else if file_type.is_symlink() {
            self.backup_symlink(path, top_level, metadata)?;
        } else if !top_level && (
            file_type.is_block_device() || file_type.is_char_device() ||
            file_type.is_fifo() || file_type.is_socket()
        ) {
            warn!("Skipping {:?}: unsupported file type.", path);
        } else {
            return self.handle_path_error(path, "unsupported file type");
        }

        Ok(())
    }

    fn backup_parent_directories(&mut self, path: &Path) -> GenericResult<bool> {
        let mut parent = PathBuf::new();

        for (index, part) in path.components().dropping_back(1).enumerate() {
            parent.push(part);

            match part {
                Component::RootDir if index == 0 => {
                    continue;
                },
                Component::Normal(_) if index != 0 => {
                },
                _ => {
                    self.handle_path_error(path, "invalid path")?;
                    return Ok(false);
                },
            }

            if self.root_parents.contains(&parent) {
                continue
            }

            let metadata = match fs::symlink_metadata(&parent) {
                Ok(metadata) => metadata,
                Err(err) => {
                    self.handle_path_error(path, err)?;
                    return Ok(false);
                },
            };

            if !metadata.is_dir() {
                self.handle_path_error(path, format!(
                    "{:?} has changed its type during the backup", parent))?;
                return Ok(false);
            }

            self.backup.add_directory(&parent, &metadata).map_err(|e| format!(
                "Failed to backup {:?}: {}", parent, e))?;

            self.root_parents.insert(parent.clone());
        }

        Ok(true)
    }

    fn backup_directory(&mut self, path: &Path, top_level: bool, metadata: Metadata) -> EmptyResult {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(err) => {
                return self.handle_access_error(
                    path, top_level, err, Some(ErrorKind::NotADirectory));
            },
        };

        let mut names = Vec::new();

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    return self.handle_access_error(path, top_level, err, None);
                },
            };
            names.push(entry.file_name());
        }

        if !top_level || !util::is_root_path(path) {
            self.backup.add_directory(path, &metadata).map_err(|e| format!(
                "Failed to backup {:?}: {}", path, e))?;
        }

        for name in names {
            self.backup_path(&path.join(name), false)?;
        }

        Ok(())
    }

    fn backup_file(&mut self, path: &Path, top_level: bool) -> EmptyResult {
        let mut open_options = OpenOptions::new();
        open_options.read(true).custom_flags(OFlag::O_NOFOLLOW.bits());

        let file = match open_options.open(path) {
            Ok(file) => file,
            Err(err) => {
                return self.handle_access_error(
                    path, top_level, err, Some(ErrorKind::FilesystemLoop));
            },
        };

        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                return self.handle_access_error(path, top_level, err, None);
            },
        };

        if !metadata.is_file() {
            return self.handle_type_change(path, top_level);
        }

        let hard_links = metadata.nlink();
        if hard_links > 1 {
            warn!("{:?} has {} hard links.", path, hard_links - 1);
        }

        Ok(self.backup.add_file(path, &metadata, file).map_err(|e| format!(
            "Failed to backup {:?}: {}", path, e))?)
    }

    fn backup_symlink(&mut self, path: &Path, top_level: bool, metadata: Metadata) -> EmptyResult {
        let target = match fs::read_link(path) {
            Ok(target) => target,
            Err(err) => {
                return self.handle_access_error(
                    path, top_level, err, Some(ErrorKind::InvalidInput));
            },
        };

        Ok(self.backup.add_symlink(path, &metadata, &target).map_err(|e| format!(
            "Failed to backup {:?}: {}", path, e))?)
    }

    fn handle_access_error(
        &mut self, path: &Path, top_level: bool, err: io::Error, type_change_kind: Option<ErrorKind>,
    ) -> EmptyResult {
        if matches!(type_change_kind, Some(kind) if kind == err.kind()) {
            return self.handle_type_change(path, top_level);
        }

        if err.kind() == ErrorKind::NotFound && !top_level {
            warn!("Failed to backup {:?}: it was deleted during backing up.", path);
            return Ok(());
        }

        self.handle_path_error(path, err)
    }

    fn handle_type_change(&mut self, path: &Path, top_level: bool) -> EmptyResult {
        // We can't save format_args!() result to a variable, so have to use closure
        let mut handle = |message| -> EmptyResult {
            if top_level {
                self.handle_error(message)
            } else {
                warn!("{}.", message);
                Ok(())
            }
        };
        handle(format_args!("Skipping {:?}: it changed its type during backing up", path))
    }

    fn handle_path_error<E: Into<GenericError>>(&mut self, path: &Path, err: E) -> EmptyResult {
        self.handle_error(format_args!("Failed to backup {:?}: {}", path, err.into()))
    }

    fn handle_error(&mut self, message: std::fmt::Arguments) -> EmptyResult {
        error!("{}.", message);
        self.ok = false;
        Ok(())
    }
}