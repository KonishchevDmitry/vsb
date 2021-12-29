use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, Component};

use bzip2::Compression;
use bzip2::write::BzEncoder;
use rayon::prelude::*;
use tar::Header;

use crate::config::BackupConfig;
use crate::core::{EmptyResult, GenericResult};
use crate::metadata::{MetadataItem, MetadataWriter};
use crate::storage::{Storage, Backup};

use super::file_reader::FileReader;

pub struct BackupFile {
    // storage: Storage, // FIXME(konishchev): Ref counter
    #[allow(dead_code)] // FIXME(konishchev): Drop
    metadata: MetadataWriter,
    data: tar::Builder<Box<dyn Write>>,
}

// FIXME(konishchev): Cleanup on error
// FIXME(konishchev): Fsync
// FIXME(konishchev): Mark broken on error
impl BackupFile {
    pub fn create(config: &BackupConfig, storage: Storage) -> GenericResult<BackupFile> {
        let (group, backup) = storage.create_backup(config.max_backups)?;

        // FIXME(konishchev): Load metadata
        group.backups.par_iter().enumerate().for_each(|(_index, _backup): (usize, &Backup)| {
        });

        let metadata_path = Path::new(&backup.path).join(Backup::METADATA_NAME);
        let metadata = MetadataWriter::new(File::create(metadata_path)?);

        let data_path = Path::new(&backup.path).join(Backup::DATA_NAME);
        let data_file = File::create(data_path)?;
        let data_writer: Box<dyn Write> = Box::new(BzEncoder::new(data_file, Compression::best()));
        let data = tar::Builder::new(data_writer);

        Ok(BackupFile {metadata, data})
    }

    pub fn add_directory(&mut self, path: &Path, metadata: &fs::Metadata) -> EmptyResult {
        let mut header = tar_header(metadata);
        Ok(self.data.append_data(&mut header, tar_path(path)?, io::empty())?)
    }

    // FIXME(konishchev): Handle file truncation properly
    pub fn add_file(&mut self, path: &Path, fs_metadata: &fs::Metadata, mut file: File) -> EmptyResult {
        let mut file_reader = FileReader::new(&mut file, fs_metadata.len());

        let mut header = tar_header(fs_metadata);
        self.data.append_data(&mut header, tar_path(path)?, &mut file_reader)?;
        let (bytes_read, hash) = file_reader.consume();

        let metadata = MetadataItem::new(path, fs_metadata, bytes_read, hash, true)?;
        self.metadata.write(&metadata)?;

        Ok(())
    }
}

fn tar_path(path: &Path) -> GenericResult<&Path> {
    Ok(path.strip_prefix(Component::RootDir).map_err(|_| format!(
        "An attempt to add an invalid path to data archive: {:?}", path))?)
}

fn tar_header(metadata: &fs::Metadata) -> Header {
    let mut header = Header::new_gnu();
    header.set_metadata(metadata);
    header
}