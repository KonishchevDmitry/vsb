use lazy_static::lazy_static;
use regex::{self, Regex};

use crate::provider::{ProviderType, FileType};

pub struct BackupFileTraits {
    pub type_: FileType,
    pub extension: &'static str,
    pub name_re: Regex,
}

impl BackupFileTraits {
    pub fn get_for(provider_type: ProviderType) -> &'static BackupFileTraits {
        lazy_static! {
            static ref LOCAL_TRAITS: BackupFileTraits = BackupFileTraits {
                type_: FileType::Directory,
                extension: "",
                name_re: BackupFileTraits::get_name_re(""),
            };

            static ref CLOUD_TRAITS: BackupFileTraits = BackupFileTraits {
                type_: FileType::File,
                extension: ".tar.gpg",
                name_re: BackupFileTraits::get_name_re(".tar.gpg"),
            };
        }

        match provider_type {
            ProviderType::Local => &LOCAL_TRAITS,
            ProviderType::Cloud => &CLOUD_TRAITS,
        }
    }

    fn get_name_re(extension: &str) -> Regex {
        let regex = r"^(\d{4}\.\d{2}\.\d{2}-\d{2}:\d{2}:\d{2})".to_owned()
            + &regex::escape(extension) + "$";
        Regex::new(&regex).unwrap()
    }
}