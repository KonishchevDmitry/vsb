use std::cell::Cell;
use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use log::{info, warn, error};
use nix::fcntl::OFlag;

use crate::backuper::backup::BackupFile;
use crate::config::{BackupConfig, BackupItemConfig};
use crate::core::{EmptyResult, GenericResult};

pub struct Backuper {
    items: Vec<BackupItemConfig>,
    backup: BackupFile,
    // FIXME(konishchev): Drop Cell?
    result: Cell<Result<(), ()>>,
}

impl Backuper {
    pub fn new(config: &BackupConfig, backup: BackupFile) -> GenericResult<Backuper> {
        let items = config.items.clone().ok_or(
            "Backup items aren't configured for the specified backup")?;
        Ok(Backuper {items, backup, result: Cell::new(Ok(()))})
    }

    // FIXME(konishchev): Implement + fsync
    pub fn run(mut self) -> Result<(), ()> {
        // FIXME(konishchev): Drop clone
        for item in &self.items.clone() {
            // FIXME(konishchev): To path?
            let path = Path::new(&item.path);
            // FIXME(konishchev): unwrap
            self.backup_path(path, true).unwrap();
        }

        // FIXME(konishchev): unwrap
        self.backup.finish().unwrap();

        self.result.get()
    }

    fn backup_path(&mut self, path: &Path, top_level: bool) -> EmptyResult {
        info!("Backing up {:?}...", path);

        if let Err(err) = crate::metadata::validate_path(path) {
            self.handle_error(format_args!("Failed to backup {:?}: {}", path, err));
            return Ok(());
        }

        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) => {
                self.handle_access_error(path, top_level, err, None);
                return Ok(());
            },
        };

        let file_type = metadata.file_type();

        if file_type.is_file() {
            self.backup_file(path, top_level)?;
        } else if file_type.is_dir() {
            self.backup_directory(path, top_level, metadata)?;
        } else if file_type.is_symlink() {
            self.backup_symlink(path, top_level, metadata)?;
        } else {
            // FIXME(konishchev): Support
            unimplemented!();
        }

        Ok(())
    }

    fn backup_directory(&mut self, path: &Path, top_level: bool, metadata: Metadata) -> EmptyResult {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(err) => {
                self.handle_access_error(path, top_level, err, Some(ErrorKind::NotADirectory));
                return Ok(());
            },
        };

        let mut names = Vec::new();

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    self.handle_access_error(path, top_level, err, None);
                    return Ok(());
                },
            };
            names.push(entry.file_name());
        }

        self.backup.add_directory(path, &metadata).map_err(|e| format!(
            "Failed to backup {:?}: {}", path, e))?;

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
                self.handle_access_error(path, top_level, err, Some(ErrorKind::FilesystemLoop));
                return Ok(());
            },
        };

        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                self.handle_access_error(path, top_level, err, None);
                return Ok(());
            },
        };

        if !metadata.is_file() {
            self.handle_type_change(path, top_level);
            return Ok(());
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
                self.handle_access_error(path, top_level, err, Some(ErrorKind::InvalidInput));
                return Ok(());
            },
        };

        Ok(self.backup.add_symlink(path, &metadata, &target).map_err(|e| format!(
            "Failed to backup {:?}: {}", path, e))?)
    }

    fn handle_access_error(
        &self, path: &Path, top_level: bool, err: io::Error, type_change_kind: Option<ErrorKind>,
    ) {
        if matches!(type_change_kind, Some(kind) if kind == err.kind()) {
            return self.handle_type_change(path, top_level);
        }

        if err.kind() == ErrorKind::NotFound && !top_level {
            return warn!("Failed to backup {:?}: it was deleted during backing up.", path);
        }

        self.handle_error(format_args!("Failed to backup {:?}: {}", path, err));
    }

    fn handle_type_change(&self, path: &Path, top_level: bool) {
        // We can't save format_args!() result to a variable, so have to use closure
        let handle = |message| {
            if top_level {
                self.handle_error(message);
            } else {
                warn!("{}.", message);
            }
        };
        handle(format_args!("Skipping {:?}: it changed its type during backing up", path))
    }

    fn handle_error(&self, message: std::fmt::Arguments) {
        error!("{}.", message);
        self.result.set(Err(()));
    }
}

// FIXME(konishchev): Implement
/*
    def __backup_path(self, path, filters, toplevel):
        try:
            elif stat.S_ISSOCK(stat_info.st_mode):
                LOG.info("Skip UNIX socket - '%s'.", path)
            else:
                self.__backup.add_file(
                    path, stat_info, link_target = link_target)

            if stat.S_ISDIR(stat_info.st_mode):
                prefix = toplevel + os.path.sep

                for filename in os.listdir(path):
                    file_path = os.path.join(path, filename)

                    for allow, regex in filters:
                        if not file_path.startswith(prefix):
                            raise LogicalError()

                        if regex.search(file_path[len(prefix):]):
                            if allow:
                                self.__backup_path(file_path, filters, toplevel)
                            else:
                                LOG.info("Filtering out '%s'...", file_path)

                            break
                    else:
                        self.__backup_path(file_path, filters, toplevel)

 */