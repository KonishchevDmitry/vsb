use std::fs::DirBuilder;
use std::io::{self, ErrorKind};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf, Component};

use crate::core::{EmptyResult, GenericResult};
use crate::util;

pub fn get_file_path_from_tar_path<P: AsRef<Path>>(tar_path: P) -> GenericResult<PathBuf> {
    let tar_path = tar_path.as_ref();
    let mut path = PathBuf::from("/");

    let mut changed = false;

    for part in tar_path.components() {
        if let Component::Normal(part) = part {
            path.push(part);
            changed = true;
        } else {
            return Err!("Got an invalid file path from archive: {:?}", tar_path);
        }
    }

    if !changed {
        return Err!("Got an invalid file path from archive: {:?}", tar_path);
    }

    Ok(path)
}

pub fn get_restore_path<R, P>(restore_dir: R, file_path: P) -> GenericResult<PathBuf>
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

pub fn create_directory<P: AsRef<Path>>(path: P) -> EmptyResult {
    let path = path.as_ref();
    Ok(create_directory_inner(path).map_err(|e| format!(
        "Unable to create {:?}: {}", path, e))?)
}

fn create_directory_inner<P: AsRef<Path>>(path: P) -> io::Result<()> {
    DirBuilder::new().mode(0o700).create(path)
}

pub fn restore_directories<R, P>(restore_dir: R, file_path: P) -> GenericResult<Vec<PathBuf>>
    where R: AsRef<Path>, P: AsRef<Path>
{
    let mut path = file_path.as_ref();
    let mut restored = Vec::new();
    let mut to_create = Vec::new();

    loop {
        path = path.parent().ok_or_else(|| format!(
            "Invalid restoring file path: {:?}", file_path.as_ref()))?;

        if util::is_root_path(path) {
            break;
        }

        let restore_path = get_restore_path(restore_dir.as_ref(), path)?;

        match create_directory_inner(&restore_path) {
            Ok(_) => {
                restored.push(path.to_owned());
                break;
            },
            Err(err) => match err.kind() {
                ErrorKind::NotFound => {
                    restored.push(path.to_owned());
                    to_create.push(restore_path);
                },
                ErrorKind::AlreadyExists => {
                    break;
                }
                _ => return Err!("Unable to create {:?}: {}", restore_path, err),
            }
        }
    }

    for path in to_create.iter().rev() {
        create_directory(path)?;
    }

    Ok(restored)
}