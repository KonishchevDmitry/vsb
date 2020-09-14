use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};

use bzip2::read::BzDecoder;

use crate::core::GenericResult;
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
    pub error_files: usize,
}

pub struct BackupOuterStat {
    pub metadata_size: u64,
    pub data_size: u64,
}

impl Backup {
    pub fn read(provider: &dyn ReadProvider, name: &str, path: &str, archive: bool) -> GenericResult<Backup> {
        let mut backup = Backup {
            path: path.to_owned(),
            name: name.to_owned(),
            metadata_path: None,
            inner_stat: None,
            outer_stat: None,
        };

        if archive {
            return Ok(backup)
        }

        let backup_files: HashMap<String, Option<u64>> = provider.list_directory(path)?
            .ok_or_else(|| "The backup doesn't exist")?
            .drain(..)
            .filter(|file| file.type_ == FileType::File)
            .map(|file| (file.name, file.size))
            .collect();

        let metadata_name = "metadata.bz2";
        let metadata_size = if let Some(size) = backup_files.get(metadata_name).copied() {
            backup.metadata_path.replace(format!("{}/{}", path, metadata_name));
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
        let metadata_path = self.metadata_path.as_ref().ok_or_else(||
            "The backup has no metadata file")?;

        if cfg!(debug_assertions) {
            warn!("Skip consistency check of {:?}: running in develop mode.", metadata_path);
            return Ok(true);
        }

        let metadata_file = provider.open_file(&metadata_path)
            .map(BzDecoder::new).map(BufReader::new)
            .map_err(|e| format!("Unable to open metadata file: {}", e))?;

        let mut stat = BackupInnerStat {
            extern_files: 0,
            unique_files: 0,
            error_files:  0,
        };

        for line in metadata_file.lines() {
            let line = line.map_err(|e| format!("Error while reading metadata file: {}", e))?;

            let mut parts = line.splitn(4, ' ');
            let checksum = parts.next();
            let status = parts.next();
            let fingerprint = parts.next();
            let filename = parts.next();

            let (checksum, unique, filename) = match (checksum, status, fingerprint, filename) {
                (Some(checksum), Some(status), Some(_), Some(filename))
                if status == "extern" || status == "unique" => (checksum, status == "unique", filename),
                _ => return Err!("Error while reading metadata file: it has an unsupported format"),
            };

            if unique {
                stat.unique_files += 1;
                available_checksums.insert(checksum.to_owned());
            } else {
                stat.extern_files += 1;

                if !available_checksums.contains(checksum) {
                    stat.error_files += 1;

                    error!(concat!(
                        "{:?} backup on {} is not recoverable: ",
                        "unable to find extern {:?} file in the backup group."
                    ), self.name, provider.name(), filename);
                }
            }
        }

        let has_files = stat.unique_files != 0 || stat.extern_files != 0;
        if !has_files {
            error!("{:?} backup on {} don't have any files.", self.name, provider.name());
        }

        let recoverable = stat.error_files == 0;
        self.inner_stat.replace(stat);

        Ok(has_files && recoverable)
    }
}