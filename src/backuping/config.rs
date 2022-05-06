use std::path::PathBuf;

use serde_derive::Deserialize;

use crate::core::GenericResult;

use super::filter::PathFilter;

// FIXME(konishchev): Rewrite
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupItemConfig {
    pub path: String,
    #[serde(default)]
    pub filter: PathFilter,
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