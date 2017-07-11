use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf, Component};

use clap::{App, Arg, AppSettings};
use shellexpand;
use serde_yaml;

use core::GenericResult;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub backups: Vec<Backup>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    pub name: String,
    pub src: String,
    pub dst: String,
    pub provider: Provider,
    pub max_backup_groups: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "name")]
pub enum Provider {
    #[serde(rename = "dropbox")]
    Dropbox {access_token: String},
}

pub fn load() -> GenericResult<Config> {
    let default_config_path = "~/.pyvsb_to_cloud.yaml";

    let matches = App::new("PyVSB to cloud")
        .about("\nUploads PyVSB backups to cloud")
        .arg(Arg::with_name("config")
            .short("c")
            .long("config")
            .value_name("PATH")
            .help(&format!("Configuration file path [default: {}]", default_config_path))
            .takes_value(true))
        .setting(AppSettings::DisableVersion)
        .get_matches();

    let config_path = matches.value_of("config").map(ToString::to_string).unwrap_or_else(||
        shellexpand::tilde(default_config_path).to_string());

    Ok(load_config(&config_path).map_err(|e| format!(
        "Failed to read {:?} configuration file: {}", config_path, e))?)
}

fn load_config(path: &str) -> GenericResult<Config> {
    let mut data = Vec::new();
    File::open(path)?.read_to_end(&mut data)?;
    let mut config: Config = serde_yaml::from_slice(&data)?;

    for backup in config.backups.iter_mut() {
        backup.name = validate_name(&backup.name)?;
        backup.src = validate_path(&shellexpand::tilde(&backup.src))?;
        backup.dst = validate_path(&backup.dst)?;

        if backup.max_backup_groups == 0 {
            return Err!("Maximum backup groups number must be positive")
        }
    }

    Ok(config)
}

fn validate_name(mut name: &str) -> GenericResult<String> {
    name = name.trim();
    if name.len() == 0 {
        return Err!("Backup name mustn't be empty");
    }
    Ok(name.to_owned())
}

fn validate_path(path: &str) -> GenericResult<String> {
    let mut normalized_path = PathBuf::new();
    let mut path_components = Path::new(path).components();

    if path_components.next() != Some(Component::RootDir) {
        return Err!("Backup paths must be absolute");
    }
    normalized_path.push(Component::RootDir.as_os_str());

    for component in path_components {
        if let Component::Normal(component) = component {
            normalized_path.push(component);
        } else {
            return Err!("Invalid path: {}", path);
        }
    }

    Ok(normalized_path.to_str().unwrap().to_owned())
}