use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf, Component};

use serde_derive::Deserialize;
use validator::Validate;

use crate::core::GenericResult;

pub use crate::backuping::BackupConfig;
pub use crate::backuping::BackupItemConfig;
pub use crate::uploading::UploadConfig;

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub path: PathBuf,
    #[validate]
    #[serde(default)]
    pub backups: Vec<BackupSpecConfig>,
    #[validate(length(min = 1))]
    pub prometheus_metrics: Option<String>,
}

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct BackupSpecConfig {
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub path: String,
    #[validate]
    pub backup: Option<BackupConfig>,
    #[validate]
    pub upload: Option<UploadConfig>,
}

impl Config {
    pub fn load(path: &Path) -> GenericResult<Config> {
        let mut data = Vec::new();
        File::open(path)?.read_to_end(&mut data)?;

        let mut config: Config = serde_yaml::from_slice(&data)?;
        config.path = path.to_owned();
        config.validate()?;

        let mut backup_names = HashSet::new();

        for backup in config.backups.iter_mut() {
            if !backup_names.insert(&backup.name) {
                return Err!("Duplicated backup name: {:?}", backup.name);
            }

            backup.path = validate_local_path(&backup.path)?;
            if let Some(upload) = backup.upload.as_mut() {
                upload.path = validate_path(&upload.path)?;
            }
        }

        if let Some(metrics_path) = config.prometheus_metrics.as_mut() {
            *metrics_path = validate_local_path(metrics_path)?;
        }

        Ok(config)
    }

    pub fn get_backup(&self, name: &str) -> GenericResult<&BackupSpecConfig> {
        for backup in &self.backups {
            if backup.name == name {
                return Ok(backup);
            }
        }

        Err!("{:?} backup is not specified in the configuration file", name)
    }
}

fn validate_path(path: &str) -> GenericResult<String> {
    let mut normalized_path = PathBuf::new();
    let mut path_components = Path::new(path).components();

    if path_components.next() != Some(Component::RootDir) {
        return Err!("Invalid path: {:?}. It must be absolute", path);
    }
    normalized_path.push(Component::RootDir.as_os_str());

    for component in path_components {
        if let Component::Normal(component) = component {
            normalized_path.push(component);
        } else {
            return Err!("Invalid path: {:?}", path);
        }
    }

    Ok(normalized_path.to_str().unwrap().to_owned())
}

fn validate_local_path(path: &str) -> GenericResult<String> {
    validate_path(&shellexpand::tilde(path))
}