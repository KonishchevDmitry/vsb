// FIXME(konishchev): Handle file truncation during backing up properly

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf, Component};

use itertools::Itertools;
use serde_json::Value;
use tar::{Archive, Header, Entry, EntryType};

use crate::core::{EmptyResult, GenericError, GenericResult};
use crate::hash::Hash;
use crate::providers::filesystem::Filesystem;
use crate::storage::{Storage, StorageRc, BackupGroup, Backup, BackupTraits};

pub struct Restorer {
    storage: StorageRc,
    plan: RestorePlan,
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

        let plan = RestorePlan::new(&storage, group_name, backup_name)?;
        Ok(Restorer {storage, plan})
    }

    // FIXME(konishchev): Rewrite
    pub fn restore(&self, group_name: &str, backup_name: &str, path: &Path) -> EmptyResult {
        let group_path = self.storage.get_backup_group_path(group_name);

        // FIXME(konishchev): ok
        let (mut group, _) = BackupGroup::read(self.storage.provider.read(), group_name, &group_path, true)?;

        // FIXME(konishchev): Unwrap
        let (position, _) = group.backups.iter().find_position(|backup| backup.name == backup_name).unwrap();
        let tail_size = group.backups.len() - position - 1;

        let mut extern_files: HashMap<Hash, Vec<String>> = HashMap::new();
        let mut restore_plan = Vec::new();

        for (index, backup) in group.backups.into_iter().rev().dropping(tail_size).enumerate() {
            if index == 0 {
                for file in backup.read_metadata(self.storage.provider.read())? {
                    let file = file?;
                    if !file.unique {
                        extern_files.entry(file.hash).or_default().push(file.path);
                    }
                }

                restore_plan.push(RestoreStepOld {backup, files: HashMap::new()})
            } else {
                if extern_files.is_empty() {
                    break;
                }

                let mut files = HashMap::new();

                for file in backup.read_metadata(self.storage.provider.read())? {
                    let file = file?;
                    if let Some(paths) = extern_files.remove(&file.hash) {
                        files.insert(file.path, paths);
                    }
                }

                if !files.is_empty() {
                    restore_plan.push(RestoreStepOld {backup, files})
                }
            }
        }

        if !extern_files.is_empty() {
            // FIXME(konishchev): Support
            unimplemented!();
        }

        self.restore_impl(restore_plan, path)?;
        Ok(())
    }

    // FIXME(konishchev): Rewrite
    // FIXME(konishchev): Implement
    fn restore_impl(&self, plan: Vec<RestoreStepOld>, path: &Path) -> EmptyResult {
        assert!(!plan.is_empty());

        let step = plan.first().unwrap();
        let mut archive = step.backup.read_data(self.storage.provider.read())?;

        fs::create_dir(path).map_err(|e| format!("Failed to create {:?}: {}", path, e))?;

        // let mut directories = Vec::new();
        // for entry in archive.entries()? {
        //     let entry = entry?;
            //     let mut file = entry.map_err(|e| TarError::new("failed to iterate over archive", e))?;
        //
        //     if file.header().entry_type() == crate::EntryType::Directory {
        //         directories.push(file);
        //     } else {
        //         file.unpack_in(dst)?;
        //     }
        // }
        // for mut dir in directories {
        //     dir.unpack_isn(dst)?;
        // }

        Ok(())
    }

    // FIXME(konishchev): Implement
    fn restore_target_backup(&mut self, mut archive: Archive<Box<dyn Read>>, restore_dir: &Path) -> EmptyResult {
        let mut created_directories = HashMap::new(); // FIXME(konishchev): Implement

        for entry in archive.entries()? {
            let entry = entry?;
            let path = entry.path()?;
            let header = entry.header();

            let restore_path = get_restore_path(restore_dir, &path)?;
            let permissions = self.get_permissions(header)?;

            match header.entry_type() {
                EntryType::Directory => {
                    fs::create_dir(&restore_path).map_err(|e| format!(
                        "Unable to create {:?}: {}", restore_path, e))?;

                    created_directories.insert(restore_path, permissions);
                }
                EntryType::Regular => {
                    // OpenOptions::new().write(true).create_new(true).open(dst)

                },
                _ => {
                    // FIXME(konishchev): Support
                    return Err!("Got an unsupported entry: {:?}", entry.path())
                }
            }
        }

        Ok(())
    }

    // FIXME(konishchev): Implement
    fn get_permissions(&mut self, header: &Header) -> GenericResult<Permissions> {
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

fn get_restore_path(restore_dir: &Path, file_path: &Path) -> GenericResult<PathBuf> {
    let mut path = restore_dir.to_path_buf();
    let mut changed = false;

    for (index, part) in file_path.components().enumerate() {
        match part {
            Component::RootDir if index == 0 => {},
            Component::Normal(part) if index != 0 => {
                path.push(part);
                changed = true;
            },
            _ => return Err!("Got an invalid file path from archive: {:?}", file_path),
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