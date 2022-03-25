use std::fs::{self, Permissions};
use std::os::unix::{self, fs::PermissionsExt};
use std::path::Path;

use filetime::FileTime;

use crate::core::EmptyResult;

#[derive(Clone, Copy)]
pub struct FileMetadata {
    owner: Option<Owner>,
    mode: u32,
    mtime: i64,
}

#[derive(Clone, Copy)]
pub struct Owner {
    uid: u32,
    gid: u32,
}

impl FileMetadata {
    pub fn set(&self, path: &Path) -> EmptyResult {
        if let Some(owner) = self.owner {
            unix::fs::chown(path, Some(owner.uid), Some(owner.gid)).map_err(|e| format!(
                "Unable to change {:?} ownership: {}", path, e))?;
        };

        fs::set_permissions(path, Permissions::from_mode(self.mode)).map_err(|e| format!(
            "Unable to change {:?} permissions: {}", path, e))?;

        let time = FileTime::from_unix_time(self.mtime, 0);
        filetime::set_symlink_file_times(path, time, time).map_err(|e| format!(
            "Unable to change {:?} modification time: {}", path, e))?;

        Ok(())
    }
}