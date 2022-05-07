use std::fmt;
use std::time::Duration;

use lazy_static::lazy_static;
use regex::Regex;
use serde::de::{self, Deserializer, Visitor};
use serde_derive::Deserialize;
use validator::Validate;

use crate::core::GenericResult;

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct UploadConfig {
    pub provider: ProviderConfig,
    #[validate(length(min = 1))]
    pub path: String,
    #[validate(range(min = 1))]
    pub max_backup_groups: usize,
    #[validate(length(min = 1))]
    pub encryption_passphrase: String,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_duration")]
    pub max_time_without_backups: Option<Duration>,
}

#[derive(Deserialize)]
#[serde(tag = "name")]
pub enum ProviderConfig {
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