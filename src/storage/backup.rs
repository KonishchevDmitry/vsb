use std::collections::HashSet;
use std::io::{BufRead, BufReader};

use bzip2::read::BzDecoder;

use crate::core::GenericResult;
use crate::provider::ReadProvider;


pub struct Backup {
    pub path: String,
    pub name: String,
    metadata_path: Option<String>,
    stat: Option<BackupStat>,
}

pub struct BackupStat {
    pub extern_files: usize,
    pub unique_files: usize,
    pub error_files: usize,
}

impl Backup {
    pub fn read(provider: &dyn ReadProvider, name: &str, path: &str, archive: bool) -> GenericResult<Backup> {
        let mut backup = Backup {
            path: path.to_owned(),
            name: name.to_owned(),
            metadata_path: None,
            stat: None
        };

        if archive {
            return Ok(backup)
        }

        let backup_files: HashSet<String> = provider.list_directory(path)?
            .ok_or_else(|| "The backup doesn't exist".to_owned())?
            .drain(..).map(|file| file.name).collect();

        let metadata_name = "metadata.bz2";
        if !backup_files.contains(metadata_name) {
            return Err!("The backup is corrupted: metadata file is missing")
        }
        backup.metadata_path.replace(format!("{}/{}", path, metadata_name));

        let data_files: HashSet<String> = ["data.tar.gz", "data.tar.bz2", "data.tar.7z"]
            .iter().map(|&s| s.to_owned()).collect();

        if backup_files.is_disjoint(&data_files) {
            return Err!("The backup is corrupted: backup data file is missing")
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

        let mut stat = BackupStat {
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
        self.stat.replace(stat);

        Ok(has_files && recoverable)
    }
}