use std::collections::{HashMap, HashSet};

use log::{warn, error};

use crate::core::GenericResult;
use crate::metadata::MetadataReader;
use crate::provider::{ReadProvider, FileType};


pub struct Backup {
    pub path: String,
    pub name: String,
    metadata_path: Option<String>,
    pub inner_stat: Option<BackupInnerStat>,
    pub outer_stat: Option<BackupOuterStat>,
}

pub struct BackupInnerStat {
    pub extern_files: usize,
    pub unique_files: usize,
    pub extern_size: u64,
    pub unique_size: u64,
}

pub struct BackupOuterStat {
    pub metadata_size: u64,
    pub data_size: u64,
}

impl Backup {
    pub const NAME_FORMAT: &'static str = "%Y.%m.%d-%H:%M:%S";
    pub const METADATA_NAME: &'static str = "metadata.bz2";
    pub const DATA_NAME: &'static str = "data.tar.bz2"; // FIXME(konishchev): Variations

    pub fn new(path: &str, name: &str) -> Backup {
        Backup {
            path: path.to_owned(),
            name: name.to_owned(),
            metadata_path: None,
            inner_stat: None,
            outer_stat: None,
        }
    }

    pub fn read(provider: &dyn ReadProvider, name: &str, path: &str, archive: bool) -> GenericResult<Backup> {
        let mut backup = Backup::new(path, name);

        if archive {
            return Ok(backup)
        }

        let backup_files: HashMap<String, Option<u64>> = provider.list_directory(path)?
            .ok_or("The backup doesn't exist")?
            .drain(..)
            .filter(|file| file.type_ == FileType::File)
            .map(|file| (file.name, file.size))
            .collect();

        let metadata_size = if let Some(size) = backup_files.get(Backup::METADATA_NAME).copied() {
            backup.metadata_path.replace(format!("{}/{}", path, Backup::METADATA_NAME));
            size
        } else {
            return Err!("The backup is corrupted: metadata file is missing");
        };

        let mut has_data = false;
        let mut data_size = None;

        for &data_name in &["data.tar.gz", "data.tar.bz2", "data.tar.7z"] {
            if let Some(size) = backup_files.get(data_name).copied() {
                data_size = size;
                has_data = true;
                break;
            }
        }
        if !has_data {
            return Err!("The backup is corrupted: backup data file is missing")
        }

        if let (Some(metadata_size), Some(data_size)) = (metadata_size, data_size) {
            backup.outer_stat.replace(BackupOuterStat {metadata_size, data_size});
        }

        Ok(backup)
    }

    pub fn inspect(
        &mut self, provider: &dyn ReadProvider, available_checksums: &mut HashSet<String>,
    ) -> GenericResult<bool> {
        let metadata_path = self.metadata_path.as_ref().ok_or(
            "The backup has no metadata file")?;

        if cfg!(debug_assertions) {
            warn!("Skip consistency check of {:?}: running in develop mode.", metadata_path);
            return Ok(true);
        }

        let metadata_file = provider.open_file(metadata_path).map_err(|e| format!(
            "Unable to open metadata file: {}", e))?;

        let mut recoverable = true;
        let mut stat = BackupInnerStat {
            extern_files: 0,
            unique_files: 0,
            extern_size: 0,
            unique_size: 0,
        };

        for file in MetadataReader::new(metadata_file) {
            let file = file.map_err(|e| format!("Error while reading metadata file: {}", e))?;

            if file.unique {
                stat.unique_files += 1;
                stat.unique_size += file.size;
                available_checksums.insert(file.checksum);
            } else {
                stat.extern_files += 1;
                stat.extern_size += file.size;

                if !available_checksums.contains(&file.checksum) {
                    error!(concat!(
                        "{:?} backup on {} is not recoverable: ",
                        "unable to find extern {:?} file in the backup group."
                    ), self.name, provider.name(), file.path);
                    recoverable = false;
                }
            }
        }

        let has_files = stat.unique_files != 0 || stat.extern_files != 0;
        if !has_files {
            error!("{:?} backup on {} don't have any files.", self.name, provider.name());
        }
        self.inner_stat.replace(stat);

        Ok(has_files && recoverable)
    }
}