use const_format::concatcp;
use lazy_static::lazy_static;
use regex::{self, Regex};

use crate::providers::{ProviderType, FileType};

const DAY_PRECISION_NAME_FORMAT: &str = "%Y.%m.%d";
const DAY_PRECISION_NAME_REGEX: &str = r"\d{4}\.\d{2}\.\d{2}";

const SECOND_PRECISION_NAME_FORMAT: &str = concatcp!(DAY_PRECISION_NAME_FORMAT, "-%H:%M:%S");
const SECOND_PRECISION_NAME_REGEX: &str = concatcp!(DAY_PRECISION_NAME_REGEX, r"-\d{2}:\d{2}:\d{2}");

#[cfg(test)] const HIGH_PRECISION_NAME_FORMAT: &str = concatcp!(SECOND_PRECISION_NAME_FORMAT, ".%3f");
#[cfg(test)] const HIGH_PRECISION_NAME_REGEX: &str = concatcp!(SECOND_PRECISION_NAME_REGEX, r"\.\d{3}");

#[cfg(not(test))] const GROUP_NAME_FORMAT: &str = DAY_PRECISION_NAME_FORMAT;
#[cfg(test)] const GROUP_NAME_FORMAT: &str = HIGH_PRECISION_NAME_FORMAT;

#[cfg(not(test))] const GROUP_NAME_REGEX: &str = DAY_PRECISION_NAME_REGEX;
#[cfg(test)] const GROUP_NAME_REGEX: &str = HIGH_PRECISION_NAME_REGEX;

#[cfg(not(test))] const BACKUP_NAME_FORMAT: &str = SECOND_PRECISION_NAME_FORMAT;
#[cfg(test)] const BACKUP_NAME_FORMAT: &str = HIGH_PRECISION_NAME_FORMAT;

#[cfg(not(test))] const BACKUP_NAME_REGEX: &str = SECOND_PRECISION_NAME_REGEX;
#[cfg(test)] const BACKUP_NAME_REGEX: &str = HIGH_PRECISION_NAME_REGEX;

pub struct BackupTraits {
    pub file_type: FileType,
    pub temporary_prefix: &'static str,
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
            temporary_prefix: ".",
            name_format: BACKUP_NAME_FORMAT,
            name_regex: Regex::new(&format!("^(?P<name>{}){}$", BACKUP_NAME_REGEX, regex::escape(extension))).unwrap(),
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