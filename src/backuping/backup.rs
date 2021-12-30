use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf, Component};

use bzip2::Compression;
use bzip2::write::BzEncoder;
use log::{debug, error};
use rayon::prelude::*;
use tar::Header;

use crate::config::BackupConfig;
use crate::core::{EmptyResult, GenericResult};
use crate::metadata::{MetadataItem, MetadataWriter};
use crate::storage::{StorageRc, Backup};
use crate::util;

use super::file_reader::FileReader;

pub struct BackupInstance {
    path: PathBuf,
    temp_path: Option<PathBuf>,
    metadata: Option<MetadataWriter<File>>,
    data: Option<tar::Builder<BufWriter<BzEncoder<File>>>>,
}

// FIXME(konishchev): Mark broken on error
impl BackupInstance {
    pub fn create(config: &BackupConfig, storage: StorageRc) -> GenericResult<BackupInstance> {
        let (group, backup) = storage.create_backup(config.max_backups)?;
        let mut instance = BackupInstance {
            path: storage.get_backup_path(&group.name, &backup.name, false).into(),
            temp_path: Some(backup.path.into()),
            metadata: None,
            data: None
        };

        // FIXME(konishchev): Load metadata
        group.backups.par_iter().enumerate().for_each(|(_index, _backup): (usize, &Backup)| {
        });

        let backup_path = instance.temp_path.as_ref().unwrap();

        instance.metadata = Some(MetadataWriter::new(
            File::create(backup_path.join(Backup::METADATA_NAME))?
        ));

        instance.data = Some(tar::Builder::new(BufWriter::new(
            BzEncoder::new(
                File::create(backup_path.join(Backup::DATA_NAME))?,
                Compression::best(),
            )
        )));

        Ok(instance)
    }

    pub fn add_directory(&mut self, path: &Path, metadata: &fs::Metadata) -> EmptyResult {
        let mut header = tar_header(metadata);
        Ok(self.data.as_mut().unwrap().append_data(&mut header, tar_path(path)?, io::empty())?)
    }

    pub fn add_file(&mut self, path: &Path, fs_metadata: &fs::Metadata, mut file: File) -> EmptyResult {
        let mut file_reader = FileReader::new(&mut file, fs_metadata.len());

        let mut header = tar_header(fs_metadata);
        self.data.as_mut().unwrap().append_data(&mut header, tar_path(path)?, &mut file_reader)?;
        let (bytes_read, hash) = file_reader.consume();

        let metadata = MetadataItem::new(path, fs_metadata, bytes_read, hash, true)?;
        self.metadata.as_mut().unwrap().write(&metadata)?;

        Ok(())
    }

    pub fn add_symlink(&mut self, path: &Path, metadata: &fs::Metadata, target: &Path) -> EmptyResult {
        let mut header = tar_header(metadata);
        Ok(self.data.as_mut().unwrap().append_link(&mut header, tar_path(path)?, target)?)
    }

    pub fn finish(mut self) -> EmptyResult {
        debug!("Fsyncing...");

        self.metadata.take().unwrap().finish()?.sync_all()?;
        self.data.take().unwrap().into_inner()?
            .into_inner().map_err(|e| e.into_error())?.finish()?
            .sync_all()?;

        let temp_path = self.temp_path.clone().unwrap();
        let parent_path = temp_path.parent().unwrap();

        util::fsync_directory(&temp_path)?;
        fs::rename(&temp_path, &self.path)?;
        self.temp_path = None;
        util::fsync_directory(parent_path)?;

        Ok(())
    }
}

impl Drop for BackupInstance {
    fn drop(&mut self) {
        if let Some(path) = self.temp_path.take() {
            if let Err(err) = fs::remove_dir_all(&path) {
                error!("Failed to delete {:?}: {}.", path, err);
            }
        }
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