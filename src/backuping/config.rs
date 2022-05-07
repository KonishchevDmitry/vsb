use std::path::PathBuf;

use serde_derive::{Serialize, Deserialize};
use validator::Validate;

use crate::core::GenericResult;

use super::filter::PathFilter;

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    #[validate]
    #[validate(length(min = 1))]
    pub items: Vec<BackupItemConfig>,
    #[validate(range(min = 1))]
    pub max_backup_groups: usize,
    #[validate(range(min = 1))]
    pub max_backups_per_group: usize,
}

#[derive(Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct BackupItemConfig {
    #[validate(length(min = 1))]
    pub path: String,
    #[serde(default)]
    pub filter: PathFilter,
    pub before: Option<String>,
    pub after: Option<String>,
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