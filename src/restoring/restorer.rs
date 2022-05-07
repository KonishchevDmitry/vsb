use std::collections::HashSet;
use std::fmt::Display;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::{self, fs::OpenOptionsExt};
use std::path::{Path, PathBuf};

use easy_logging::GlobalContext;
use itertools::Itertools;
use log::{error, debug};
use tar::{Entry, EntryType, Header};

use crate::core::{EmptyResult, GenericResult};
use crate::providers::filesystem::Filesystem;
use crate::storage::{Storage, StorageRc};
use crate::util::file_reader::FileReader;

use super::file_metadata::{FileMetadata, Owner};
use super::multi_writer::MultiWriter;
use super::plan::{RestorePlan, RestoreStep, RestoringFile};
use super::users::UsersCache;
use super::util::{self, get_restore_path};

pub struct Restorer {
    storage: StorageRc,
    group_name: String,
    backup_name: String,

    users: Option<UsersCache>,
    pending_extern_files: HashSet<PathBuf>,
    restored_extern_files: HashSet<PathBuf>,
    missing_extern_files: HashSet<PathBuf>,
    pre_created_directories: HashSet<PathBuf>,
    scheduled_file_metadata: Vec<(PathBuf, FileMetadata)>,
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

        let storage = Storage::new_read_only(Filesystem::new(), backup_root);
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

            pending_extern_files: HashSet::new(),
            restored_extern_files: HashSet::new(),
            missing_extern_files: HashSet::new(),
            pre_created_directories: HashSet::new(),
            scheduled_file_metadata: Vec::new(),
        })
    }

    pub fn restore(mut self, restore_dir: &Path) -> GenericResult<bool> {
        let (plan, mut ok) = RestorePlan::new(&self.storage, &self.group_name, &self.backup_name)?;
        self.pending_extern_files = plan.extern_files;
        self.missing_extern_files = plan.missing_files;

        util::create_directory(restore_dir)?;

        for (index, step) in plan.steps.iter().enumerate() {
            let _context = GlobalContext::new(&step.backup.name);
            ok &= self.process_step(step, index == 0, restore_dir).map_err(|e| format!(
                "Failed to restore {:?} backup: {}", step.backup.path, e))?;
        }

        let missing_extern_data = self.pending_extern_files;
        for (path, metadata) in self.scheduled_file_metadata.iter().rev() {
            if !missing_extern_data.contains(path) {
                metadata.set(get_restore_path(restore_dir, path)?)?;
            }
        }

        if !missing_extern_data.is_empty() {
            error!("Failed to restore the following files (missing extern data):");
            for path in missing_extern_data {
                error!("* {}", path.display())
            }
            ok = false;
        }

        if !self.pre_created_directories.is_empty() {
            error!("The following directories have been unexpectedly restored without known permissions:");
            for path in self.pre_created_directories {
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
            let file_path = util::get_file_path_from_tar_path(&entry_path)?;

            match entry_type {
                EntryType::Directory => if is_target {
                    if !self.pre_created_directories.remove(&file_path) {
                        util::create_directory(get_restore_path(restore_dir, &file_path)?)?;
                    }
                    self.schedule_file_metadata_change(file_path, header)?;
                }

                EntryType::Regular => {
                    if let Some(info) = step.files.get(&file_path) {
                        self.restore_files(&file_path, entry, info, restore_dir, is_target)?;
                    } else if is_target {
                        if self.pending_extern_files.contains(&file_path) || self.restored_extern_files.contains(&file_path) {
                            if entry.size() != 0 {
                                error!("The backup archive has data for {:?} file which is expected to be external.", file_path);
                                ok = false;
                            }
                            self.schedule_file_metadata_change(file_path, header)?;
                        } else if !self.missing_extern_files.contains(&file_path) {
                            error!("The backup archive contains an unexpected {:?} file. Ignore it.", file_path);
                            ok = false;
                        }
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

    // FIXME(konishchev): Workaround too many open files here
    fn restore_files(
        &mut self, source_path: &Path, mut entry: Entry<Box<dyn Read>>, info: &RestoringFile,
        restore_dir: &Path, is_target: bool,
    ) -> EmptyResult {
        let paths = info.paths.iter().map(|path| format!("{:?}", path)).join(", ");
        debug!("Restoring {}...", paths);

        let mut files = Vec::new();
        let mut restore_metadata = None;

        for path in &info.paths {
            let restore_path = get_restore_path(restore_dir, path)?;

            if is_target {
                if path == source_path {
                    let metadata = self.get_file_metadata(entry.header())?;
                    assert!(restore_metadata.replace((restore_path.clone(), metadata)).is_none());
                } else {
                    self.pre_created_directories.extend(util::restore_directories(restore_dir, path)?);
                    self.restored_extern_files.insert(self.pending_extern_files.take(path).unwrap());
                }
            } else {
                self.restored_extern_files.insert(self.pending_extern_files.take(path).unwrap());
            }

            files.push(OpenOptions::new()
                .create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).write(true)
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

    fn schedule_file_metadata_change(&mut self, path: PathBuf, header: &Header) -> EmptyResult {
        self.scheduled_file_metadata.push((path, self.get_file_metadata(header)?));
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