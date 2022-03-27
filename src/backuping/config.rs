use std::path::PathBuf;

use serde_derive::{Serialize, Deserialize};

use crate::core::GenericResult;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupItemConfig {
    pub path: String,
}

impl BackupItemConfig {
    pub fn path(&self) -> GenericResult<PathBuf> {
        let path = expanduser::expanduser(&self.path)?;
        if !path.is_absolute() {
            return Err!("the path must be absolute");
        }
        Ok(path.canonicalize()?)
    }
}