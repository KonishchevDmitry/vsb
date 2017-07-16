use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf, Component};
use std::process;

use clap::{App, Arg, AppSettings};
use log::LogLevel;
use shellexpand;
use serde_yaml;

use core::GenericResult;
use logging;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub path: String,
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
    pub encryption_passphrase: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "name")]
pub enum Provider {
    #[serde(rename = "dropbox")]
    Dropbox {access_token: String},
}

pub fn load() -> Config {
    let default_config_path = "~/.pyvsb_to_cloud.yaml";

    let matches = App::new("PyVSB to cloud")
        .about("\nUploads PyVSB backups to cloud")
        .arg(Arg::with_name("config")
            .short("c")
            .long("config")
            .value_name("PATH")
            .help(&format!("Configuration file path [default: {}]", default_config_path))
            .takes_value(true))
        .arg(Arg::with_name("verbose")
            .short("v")
            .long("verbose")
            .multiple(true)
            .help("Sets the level of verbosity"))
        .setting(AppSettings::DisableVersion)
        .get_matches();

    let log_level = match matches.occurrences_of("verbose") {
        0 => LogLevel::Info,
        1 => LogLevel::Debug,
        2 => LogLevel::Trace,
        _ => {
            let _ = writeln!(io::stderr(), "Invalid verbosity level.");
            process::exit(1);
        }
    };

    if let Err(err) = logging::init(log_level) {
        let _ = writeln!(io::stderr(), "Failed to initialize the logging: {}.", err);
        process::exit(1);
    }

    let config_path = matches.value_of("config").map(ToString::to_string).unwrap_or_else(||
        shellexpand::tilde(default_config_path).to_string());

    match load_config(&config_path) {
        Ok(config) => config,
        Err(err) => {
            error!("Error while reading {:?} configuration file: {}.", config_path, err);
            process::exit(1);
        }
    }
}

fn load_config(path: &str) -> GenericResult<Config> {
    let mut data = Vec::new();
    File::open(path)?.read_to_end(&mut data)?;

    let mut config: Config = serde_yaml::from_slice(&data)?;
    config.path = path.to_owned();

    for backup in config.backups.iter_mut() {
        backup.name = validate_name(&backup.name)?;
        backup.src = validate_path(&shellexpand::tilde(&backup.src))?;
        backup.dst = validate_path(&backup.dst)?;

        if backup.max_backup_groups == 0 {
            return Err!("Maximum backup groups number must be positive");
        }

        if backup.encryption_passphrase.len() == 0 {
            return Err!("Encryption passphrase mustn't be empty");
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