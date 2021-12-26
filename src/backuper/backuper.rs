use std::cell::Cell;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;

use log::{info, warn, error};
use nix::fcntl::OFlag;

use crate::config::{BackupConfig, BackupItemConfig};
use crate::core::GenericResult;

pub struct Backuper {
    items: Vec<BackupItemConfig>,
    ok: Cell<bool>,
}

impl Backuper {
    pub fn new(config: &BackupConfig) -> GenericResult<Backuper> {
        let items = config.items.clone().ok_or(
            "Backup items aren't configured for the specified backup")?;
        Ok(Backuper {items, ok: Cell::new(true)})
    }

    // FIXME(konishchev): Implement
    pub fn run(&self) -> bool {
        for item in &self.items {
            self.backup_path(&item.path, true);
        }
        self.ok.get()
    }

    fn backup_path(&self, path: &str, top_level: bool) {
        info!("Backing up {:?}...", path);

        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(err) => return self.handle_read_error(path, top_level, err),
        };

        let file_type = metadata.file_type();

        if file_type.is_file() {
            self.backup_file(path, top_level);
        } else if file_type.is_dir() {
            // FIXME(konishchev): HERE
            self.backup_directory(path, top_level);
        } else if file_type.is_symlink() {
            self.backup_symlink(path, top_level);
        } else {
            unimplemented!();
        }
    }

    fn backup_directory(&self, path: &str, top_level: bool) {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(err) => return match err.kind() {
                ErrorKind::NotADirectory => self.handle_type_change(path, top_level),
                _ => self.handle_read_error(path, top_level, err),
            },
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => return self.handle_read_error(path, top_level, err),
            };

            let path = entry.path();
            let path = match path.to_str() {
                Some(path) => path,
                None => {
                    self.handle_error(format_args!("Failed to backup {:?}: invalid path", path.to_string_lossy()));
                    continue;
                },
            };

            self.backup_path(path, false);
        }
    }

    fn backup_file(&self, path: &str, top_level: bool) {
        let mut open_options = OpenOptions::new();
        open_options.read(true).custom_flags(OFlag::O_NOFOLLOW.bits());

        let file = match open_options.open(path) {
            Ok(file) => file,
            Err(err) => return match err.kind() {
                // When O_NOFOLLOW is specified, indicates that this is a symbolic link
                ErrorKind::FilesystemLoop => self.handle_type_change(path, top_level),
                _ => self.handle_read_error(path, top_level, err),
            },
        };

        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(err) => return self.handle_read_error(path, top_level, err),
        };

        if !metadata.is_file() {
            return self.handle_type_change(path, top_level);
        }

        // FIXME(konishchev): Add to backup
    }

    fn backup_symlink(&self, path: &str, top_level: bool) {
        let _target = match fs::read_link(path) {
            Ok(target) => target,
            Err(err) => return match err.kind() {
                ErrorKind::InvalidInput => self.handle_type_change(path, top_level),
                _ => self.handle_read_error(path, top_level, err),
            },
        };

        // FIXME(konishchev): Add to backup
    }

    fn handle_read_error(&self, path: &str, top_level: bool, err: io::Error) {
        if err.kind() == ErrorKind::NotFound && !top_level {
            return warn!("Failed to backup {:?}: it was deleted during backing up.", path);
        }

        return self.handle_error(format_args!("Failed to backup {:?}: {}", path, err));
    }

    fn handle_type_change(&self, path: &str, top_level: bool) {
        let mut handle = |message| {
            if top_level {
                self.handle_error(message);
            } else {
                warn!("{}.", message)
            }
        };
        handle(format_args!("Skipping {:?}: it changed its type during backing up", path));
    }

    fn handle_error(&self, message: std::fmt::Arguments) {
        error!("{}.", message);
        self.ok.set(false);
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