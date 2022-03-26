use std::fs::{self, Permissions};
use std::os::unix::{self, fs::PermissionsExt};
use std::path::Path;

use filetime::FileTime;

use crate::core::EmptyResult;

#[derive(Clone, Copy)]
pub struct FileMetadata {
    pub owner: Option<Owner>,
    pub mode: Option<u32>,
    pub mtime: i64,
}

#[derive(Clone, Copy)]
pub struct Owner {
    pub uid: u32,
    pub gid: u32,
}

impl FileMetadata {
    pub fn set<P: AsRef<Path>>(&self, path: P) -> EmptyResult {
        let path = path.as_ref();

        if let Some(owner) = self.owner {
            unix::fs::lchown(path, Some(owner.uid), Some(owner.gid)).map_err(|e| format!(
                "Unable to change {:?} ownership: {}", path, e))?;
        };

        if let Some(mode) = self.mode {
            fs::set_permissions(path, Permissions::from_mode(mode)).map_err(|e| format!(
                "Unable to change {:?} permissions: {}", path, e))?;
        }

        let time = FileTime::from_unix_time(self.mtime, 0);
        filetime::set_symlink_file_times(path, time, time).map_err(|e| format!(
            "Unable to change {:?} modification time: {}", path, e))?;

        Ok(())
    }
}