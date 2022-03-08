use const_format::concatcp;
use lazy_static::lazy_static;
use regex::{self, Regex};

use crate::provider::{ProviderType, FileType, ReadProvider};

const DAY_PRECISION_NAME_FORMAT: &'static str = "%Y.%m.%d";
const DAY_PRECISION_NAME_REGEX: &'static str = r"\d{4}\.\d{2}\.\d{2}";

const SECOND_PRECISION_NAME_FORMAT: &'static str = concatcp!(DAY_PRECISION_NAME_FORMAT, "-%H:%M:%S");
const SECOND_PRECISION_NAME_REGEX: &'static str = concatcp!(DAY_PRECISION_NAME_REGEX, r"-\d{2}:\d{2}:\d{2}");

#[cfg(test)] const HIGH_PRECISION_NAME_FORMAT: &'static str = concatcp!(SECOND_PRECISION_NAME_FORMAT, ".%3f");
#[cfg(test)] const HIGH_PRECISION_NAME_REGEX: &'static str = concatcp!(SECOND_PRECISION_NAME_REGEX, r"\.\d{3}");

#[cfg(not(test))] const GROUP_NAME_FORMAT: &'static str = DAY_PRECISION_NAME_FORMAT;
#[cfg(test)] const GROUP_NAME_FORMAT: &'static str = HIGH_PRECISION_NAME_FORMAT;

#[cfg(not(test))] const GROUP_NAME_REGEX: &'static str = DAY_PRECISION_NAME_REGEX;
#[cfg(test)] const GROUP_NAME_REGEX: &'static str = HIGH_PRECISION_NAME_REGEX;

#[cfg(not(test))] const BACKUP_NAME_FORMAT: &'static str = SECOND_PRECISION_NAME_FORMAT;
#[cfg(test)] const BACKUP_NAME_FORMAT: &'static str = HIGH_PRECISION_NAME_FORMAT;

#[cfg(not(test))] const BACKUP_NAME_REGEX: &'static str = SECOND_PRECISION_NAME_REGEX;
#[cfg(test)] const BACKUP_NAME_REGEX: &'static str = HIGH_PRECISION_NAME_REGEX;

pub struct BackupTraits {
    pub file_type: FileType,
    pub name_format: &'static str,
    pub name_regex: Regex,
    pub extension: &'static str,

    pub group_name_format: &'static str,
    pub group_name_regex: Regex,
}

impl BackupTraits {
    fn new(file_type: FileType, extension: &'static str) -> BackupTraits {
        BackupTraits {
            file_type,
            name_format: BACKUP_NAME_FORMAT,
            name_regex: Regex::new(&format!("^({}){}$", BACKUP_NAME_REGEX, regex::escape(extension))).unwrap(),
            extension: extension,

            group_name_format: GROUP_NAME_FORMAT,
            group_name_regex: Regex::new(concatcp!("^", GROUP_NAME_REGEX, "$")).unwrap(),
        }
    }

    pub fn get_for(provider_type: ProviderType) -> &'static BackupTraits {
        lazy_static! {
            static ref LOCAL_TRAITS: BackupTraits = BackupTraits::new(FileType::Directory, "");
            static ref CLOUD_TRAITS: BackupTraits = BackupTraits::new(FileType::File, ".tar.gpg");
        }

        match provider_type {
            ProviderType::Local => &LOCAL_TRAITS,
            ProviderType::Cloud => &CLOUD_TRAITS,
        }
    }
}