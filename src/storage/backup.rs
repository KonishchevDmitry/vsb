use std::collections::{HashMap, HashSet};
use std::io::{Read, BufRead, BufReader};

use log::error;
use tar::Archive;
use zstd::stream::read::Decoder;

use crate::core::GenericResult;
use crate::providers::{ReadProvider, FileType};
use crate::storage::metadata::MetadataReader;
use crate::util::hash::Hash;

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
    pub const DATA_NAME: &'static str = "data.tar.zst";
    pub const METADATA_NAME: &'static str = "metadata.zst";

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
            .into_iter()
            .filter(|file| file.type_ == FileType::File)
            .map(|file| (file.name, file.size))
            .collect();

        let data_size = *backup_files.get(Backup::DATA_NAME).ok_or(
            "The backup is corrupted: data file is missing")?;

        let metadata_size = *backup_files.get(Backup::METADATA_NAME).ok_or(
            "The backup is corrupted: metadata file is missing")?;
        backup.metadata_path.replace(format!("{}/{}", path, Backup::METADATA_NAME));

        if let (Some(metadata_size), Some(data_size)) = (metadata_size, data_size) {
            backup.outer_stat.replace(BackupOuterStat {metadata_size, data_size});
        }

        Ok(backup)
    }

    pub fn read_metadata(&self, provider: &dyn ReadProvider) -> GenericResult<MetadataReader> {
        let path = self.metadata_path.as_ref().ok_or(
            "The backup has no metadata file")?;

        let file = provider.open_file(path).map_err(|e| format!(
            "Unable to open {:?}: {}", path, e))?;

        Ok(MetadataReader::new(file))
    }

    pub fn read_data(&self, provider: &dyn ReadProvider) -> GenericResult<Archive<Box<dyn Read>>> {
        let path = format!("{}/{}", self.path, Backup::DATA_NAME);
        let file = provider.open_file(&path).map_err(|e| format!(
            "Unable to open {:?}: {}", path, e))?;

        let reader = Box::new(BufReader::with_capacity(
            Decoder::<Box<dyn BufRead>>::recommended_output_size(),
            Decoder::new(file)?,
        ));

        Ok(Archive::new(reader))
    }

    pub fn inspect(
        &mut self, provider: &dyn ReadProvider, available_hashes: &mut HashSet<Hash>,
    ) -> GenericResult<bool> {
        let mut recoverable = true;
        let mut stat = BackupInnerStat {
            extern_files: 0,
            unique_files: 0,
            extern_size: 0,
            unique_size: 0,
        };

        for file in self.read_metadata(provider)? {
            let file = file.map_err(|e| format!("Error while reading metadata file: {}", e))?;

            if file.unique {
                stat.unique_files += 1;
                stat.unique_size += file.size;
                available_hashes.insert(file.hash);
            } else {
                stat.extern_files += 1;
                stat.extern_size += file.size;

                if file.size != 0 && !available_hashes.contains(&file.hash) {
                    error!(concat!(
                        "{:?} backup{} is not recoverable: ",
                        "unable to find extern {:?} file in the backup group."
                    ), self.name, provider.clarification(), file.path);
                    recoverable = false;
                }
            }
        }

        let has_files = stat.unique_files != 0 || stat.extern_files != 0;
        if !has_files {
            error!("{:?} backup{} don't have any files.", self.name, provider.clarification());
        }
        self.inner_stat.replace(stat);

        Ok(has_files && recoverable)
    }
}