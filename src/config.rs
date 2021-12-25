use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf, Component};
use std::time::Duration;

use lazy_static::lazy_static;
use regex::Regex;
use serde::de::{self, Deserializer, Visitor};
use serde_derive::Deserialize;

use crate::core::GenericResult;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub path: String,
    pub backups: Vec<Backup>,
    pub prometheus_metrics: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    pub name: String,
    pub src: String,
    pub dst: String,
    pub provider: Provider,
    pub max_backup_groups: usize,
    pub encryption_passphrase: String,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_duration")]
    pub max_time_without_backups: Option<Duration>,
}

#[derive(Deserialize)]
#[serde(tag = "name")]
pub enum Provider {
    #[serde(rename = "dropbox")]
    Dropbox {
        /*
        How to obtain the credentials:

        open https://www.dropbox.com/developers/apps
        client_id=...     # App key
        client_secret=... # App secret

        open "https://www.dropbox.com/oauth2/authorize?client_id=$client_id&response_type=code&token_access_type=offline"
        code=...

        curl "https://api.dropbox.com/oauth2/token" -d grant_type=authorization_code -d "code=$code" -d "client_id=$client_id" -d "client_secret=$client_secret"
        refresh_token=...

        # Test access token acquiring
        curl "https://api.dropbox.com/oauth2/token" -d grant_type=refresh_token -d "refresh_token=$refresh_token" -d "client_id=$client_id" -d "client_secret=$client_secret"
         */
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },

    #[serde(rename = "google_drive")]
    GoogleDrive {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
}

impl Config {
    pub fn load(path: &str) -> GenericResult<Config> {
        let mut data = Vec::new();
        File::open(path)?.read_to_end(&mut data)?;

        let mut config: Config = serde_yaml::from_slice(&data)?;
        config.path = path.to_owned();

        for backup in config.backups.iter_mut() {
            backup.name = validate_name(&backup.name)?;
            backup.src = validate_local_path(&backup.src)?;
            backup.dst = validate_path(&backup.dst)?;

            if backup.max_backup_groups == 0 {
                return Err!("Maximum backup groups number must be positive");
            }

            if backup.encryption_passphrase.is_empty() {
                return Err!("Encryption passphrase mustn't be empty");
            }
        }

        if let Some(metrics_path) = config.prometheus_metrics.clone() {
            config.prometheus_metrics.replace(validate_local_path(&metrics_path)?);
        }

        Ok(config)
    }
}

fn validate_name(mut name: &str) -> GenericResult<String> {
    name = name.trim();
    if name.is_empty() {
        return Err!("Backup name mustn't be empty");
    }
    Ok(name.to_owned())
}

fn validate_path(path: &str) -> GenericResult<String> {
    let mut normalized_path = PathBuf::new();
    let mut path_components = Path::new(path).components();

    if path_components.next() != Some(Component::RootDir) {
        return Err!("Paths must be absolute");
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

fn validate_local_path(path: &str) -> GenericResult<String> {
    validate_path(&shellexpand::tilde(path))
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where D: Deserializer<'de>
{
    deserializer.deserialize_string(DurationVisitor)
}

struct DurationVisitor;

impl<'de> Visitor<'de> for DurationVisitor {
    type Value = Option<Duration>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("time duration in $number{m|h|d} format")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> where E: de::Error {
        match parse_duration(v) {
            Ok(duration) => Ok(Some(duration)),
            Err(err) => Err(E::custom(err))
        }
    }
}

fn parse_duration(string: &str) -> GenericResult<Duration> {
    lazy_static! {
        static ref DURATION_RE: Regex = Regex::new(
            r"^(?P<number>[1-9]\d*)(?P<unit>[mhd])$").unwrap();
    }

    let captures = DURATION_RE.captures(string).ok_or(format!(
        "Invalid time duration specification: {:?}", string))?;

    let mut duration = captures.name("number").unwrap().as_str().parse().unwrap();
    duration *= match captures.name("unit").unwrap().as_str() {
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        _ => unreachable!(),
    };

    Ok(Duration::from_secs(duration))
}