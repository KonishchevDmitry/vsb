// FIXME(konishchev): Handle file truncation during backing up properly

use std::collections::{HashMap, HashSet, hash_map::Entry as HashMapEntry};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf, Component};

use itertools::Itertools;
use log::error;
use serde_json::Value;
use tar::{Archive, Header, Entry, EntryType};

use crate::core::{EmptyResult, GenericError, GenericResult};
use crate::hash::Hash;
use crate::providers::filesystem::Filesystem;
use crate::storage::{Storage, StorageRc, BackupGroup, Backup, BackupTraits};

pub struct Restorer {
    storage: StorageRc,
    group_name: String,
    backup_name: String,
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
            let entry = entry?;
            let header = entry.header();
            let entry_path = entry.path()?;
            let entry_type = header.entry_type();

            // FIXME(konishchev): HERE
            match entry_type {
                EntryType::Directory => if is_target {
                    let restore_path = get_full_path(restore_dir, &entry_path)?;
                    fs::create_dir(&restore_path).map_err(|e| format!(
                        "Unable to create {:?}: {}", restore_path, e))?;
                    self.schedule_permissions(restore_path, header)?;
                }

                EntryType::Regular => {
                    let file_path = get_full_path("/", entry_path)?;

                    if let Some(info) = step.files.get(&file_path) {
                        self.restore_files(info)?;
                    } else if is_target {
                        // FIXME(konishchev): Ensure external
                        if entry.size() != 0 {
                            error!("The backup archive has data for {:?} file which is expected to be external.", file_path);
                            ok = false;
                        }
                    }
                },

                EntryType::Symlink => {

                },

                _ => {
                    // FIXME(konishchev): Support
                    return Err!("Got an unsupported entry ({:?}): {:?}", entry_type, entry_path)
                }
            }
        }

        Ok(ok)
    }

    // FIXME(konishchev): Implement
    fn restore_files(&mut self, info: &FileInfo) -> EmptyResult {
        // OpenOptions::new().write(true).create_new(true).open(dst)
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
        } else if kind.is_symlink() {
            let src = match self.link_name()? {
                Some(name) => name,
                None => {
                    return Err(other(&format!(
                        "hard link listed for {} but no link name found",
                        String::from_utf8_lossy(self.header.as_bytes())
                    )));
                }
            };

            if src.iter().count() == 0 {
                return Err(other(&format!(
                    "symlink destination for {} is empty",
                    String::from_utf8_lossy(self.header.as_bytes())
                )));
            }

            symlink(&src, dst)
                .or_else(|err_io| {
                    if err_io.kind() == io::ErrorKind::AlreadyExists && self.overwrite {
                        // remove dest and try once more
                        std::fs::remove_file(dst).and_then(|()| symlink(&src, dst))
                    } else {
                        Err(err_io)
                    }
                })
                .map_err(|err| {
                    Error::new(
                        err.kind(),
                        format!(
                            "{} when symlinking {} to {}",
                            err,
                            src.display(),
                            dst.display()
                        ),
                    )
                })?;
            return Ok(Unpacked::__Nonexhaustive);

            #[cfg(target_arch = "wasm32")]
            #[allow(unused_variables)]
            fn symlink(src: &Path, dst: &Path) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::Other, "Not implemented"))
            }

            #[cfg(windows)]
            fn symlink(src: &Path, dst: &Path) -> io::Result<()> {
                ::std::os::windows::fs::symlink_file(src, dst)
            }

            #[cfg(unix)]
            fn symlink(src: &Path, dst: &Path) -> io::Result<()> {
                ::std::os::unix::fs::symlink(src, dst)
            }
        } else if kind.is_pax_global_extensions()
            || kind.is_pax_local_extensions()
            || kind.is_gnu_longname()
            || kind.is_gnu_longlink()
        {
            return Ok(Unpacked::__Nonexhaustive);
        };

        let mut f = (|| -> io::Result<std::fs::File> {
            for io in self.data.drain(..) {
                match io {
                    EntryIo::Data(mut d) => {
                        let expected = d.limit();
                        if io::copy(&mut d, &mut f)? != expected {
                            return Err(other("failed to write entire file"));
                        }
                    }
                    EntryIo::Pad(d) => {
                        // TODO: checked cast to i64
                        let to = SeekFrom::Current(d.limit() as i64);
                        let size = f.seek(to)?;
                        f.set_len(size)?;
                    }
                }
            }
            Ok(f)
        })()
        .map_err(|e| {
            let header = self.header.path_bytes();
            TarError::new(
                format!(
                    "failed to unpack `{}` into `{}`",
                    String::from_utf8_lossy(&header),
                    dst.display()
                ),
                e,
            )
        })?;

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
        if self.unpack_xattrs {
            set_xattrs(self, dst)?;
        }
        return Ok(Unpacked::File(f));

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

        #[cfg(windows)]
        fn _set_perms(
            dst: &Path,
            f: Option<&mut std::fs::File>,
            mode: u32,
            _preserve: bool,
        ) -> io::Result<()> {
            if mode & 0o200 == 0o200 {
                return Ok(());
            }
            match f {
                Some(f) => {
                    let mut perm = f.metadata()?.permissions();
                    perm.set_readonly(true);
                    f.set_permissions(perm)
                }
                None => {
                    let mut perm = fs::metadata(dst)?.permissions();
                    perm.set_readonly(true);
                    fs::set_permissions(dst, perm)
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        #[allow(unused_variables)]
        fn _set_perms(
            dst: &Path,
            f: Option<&mut std::fs::File>,
            mode: u32,
            _preserve: bool,
        ) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::Other, "Not implemented"))
        }

        #[cfg(all(unix, feature = "xattr"))]
        fn set_xattrs(me: &mut EntryFields, dst: &Path) -> io::Result<()> {
            use std::ffi::OsStr;
            use std::os::unix::prelude::*;

            let exts = match me.pax_extensions() {
                Ok(Some(e)) => e,
                _ => return Ok(()),
            };
            let exts = exts
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let key = e.key_bytes();
                    let prefix = b"SCHILY.xattr.";
                    if key.starts_with(prefix) {
                        Some((&key[prefix.len()..], e))
                    } else {
                        None
                    }
                })
                .map(|(key, e)| (OsStr::from_bytes(key), e.value_bytes()));

            for (key, value) in exts {
                xattr::set(dst, key, value).map_err(|e| {
                    TarError::new(
                        format!(
                            "failed to set extended \
                             attributes to {}. \
                             Xattrs: key={:?}, value={:?}.",
                            dst.display(),
                            key,
                            String::from_utf8_lossy(value)
                        ),
                        e,
                    )
                })?;
            }

            Ok(())
        }
        // Windows does not completely support posix xattrs
        // https://en.wikipedia.org/wiki/Extended_file_attributes#Windows_NT
        #[cfg(any(windows, not(feature = "xattr"), target_arch = "wasm32"))]
        fn set_xattrs(_: &mut EntryFields, _: &Path) -> io::Result<()> {
            Ok(())
        }
 */

    Ok(())
}

fn get_full_path<B, P>(base: B, file_path: P) -> GenericResult<PathBuf>
    where B: AsRef<Path>, P: AsRef<Path>
{
    let file_path = file_path.as_ref();
    let mut path = base.as_ref().to_path_buf();

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