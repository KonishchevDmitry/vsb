use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, SeekFrom, BufWriter, Seek};
use std::path::{Path, PathBuf, Component};

use bzip2::Compression;
use bzip2::write::BzEncoder;
use log::{debug, error, warn};
use rayon::prelude::*;
use tar::Header;

use crate::config::BackupConfig;
use crate::core::{EmptyResult, GenericResult};
use crate::storage::{Storage, BackupGroup, Backup};
use crate::storage::metadata::{MetadataItem, Fingerprint, MetadataWriter};
use crate::util::{self, hash::Hash};
use crate::util::file_reader::{FileReader, EMPTY_FILE_HASH};

type Archive = tar::Builder<BufWriter<BzEncoder<File>>>;

pub struct BackupInstance {
    path: PathBuf,
    temp_path: Option<PathBuf>,

    metadata: Option<MetadataWriter<File>>,
    data: Option<Archive>,

    extern_hashes: HashSet<Hash>,
    last_state: Option<HashMap<PathBuf, FileState>>
}

impl BackupInstance {
    pub fn create(config: &BackupConfig, storage: &Storage) -> GenericResult<(BackupInstance, bool)> {
        let (group, backup) = storage.create_backup(config.max_backups)?;
        let mut instance = BackupInstance {
            path: storage.get_backup_path(&group.name, &backup.name, false).into(),
            temp_path: Some(backup.path.into()),

            metadata: None,
            data: None,

            extern_hashes: HashSet::new(),
            last_state: None,
        };

        let backup_path = instance.temp_path.as_ref().unwrap();

        let metadata_path = backup_path.join(Backup::METADATA_NAME);
        instance.metadata = Some(MetadataWriter::new(
            File::create(&metadata_path).map_err(|e| format!(
                "Failed to create {:?}: {}", metadata_path, e))?
        ));

        let data_path = backup_path.join(Backup::DATA_NAME);
        instance.data = Some(tar::Builder::new(BufWriter::new(
            BzEncoder::new(
                File::create(&data_path).map_err(|e| format!(
                    "Failed to create {:?}: {}", data_path, e))?,
                Compression::best(),
            )
        )));

        let (extern_hashes, last_state, ok) = load_backups_metadata(storage, &group);
        instance.extern_hashes = extern_hashes;
        instance.last_state = last_state;

        Ok((instance, ok))
    }

    pub fn add_directory(&mut self, path: &Path, metadata: &fs::Metadata) -> EmptyResult {
        let mut header = tar_header(metadata);
        Ok(self.data().append_data(&mut header, tar_path(path)?, io::empty())?)
    }

    pub fn add_file(&mut self, path: &Path, fs_metadata: &fs::Metadata, mut file: File) -> EmptyResult {
        let archive_path = tar_path(path)?;
        let mut header = tar_header(fs_metadata);

        let fingerprint = Fingerprint::new(fs_metadata);
        let size = fs_metadata.len();

        let (hash, size, unique) = if let Some((hash, size)) = self.deduplicate(path, &mut file, &fingerprint, size)? {
            header.set_size(0);
            self.data().append_data(&mut header, archive_path, io::empty())?;
            (hash, size, false)
        } else {
            let mut file_reader = FileReader::new(&mut file, size);
            self.data().append_data(&mut header, archive_path, &mut file_reader)?;

            let (bytes_read, hash) = file_reader.consume();
            if bytes_read != size {
                warn!("{:?} has been truncated during backup.", path);
            }

            self.extern_hashes.insert(hash.clone());
            (hash, bytes_read, true)
        };

        let metadata = MetadataItem::new(path, size, hash, fingerprint, unique)?;
        self.metadata.as_mut().unwrap().write(&metadata)?;

        Ok(())
    }

    pub fn add_symlink(&mut self, path: &Path, metadata: &fs::Metadata, target: &Path) -> EmptyResult {
        let mut header = tar_header(metadata);
        Ok(self.data().append_link(&mut header, tar_path(path)?, target)?)
    }

    pub fn finish(mut self) -> EmptyResult {
        debug!("Fsyncing...");

        self.metadata.take().unwrap().finish()?.sync_all()?;
        self.data.take().unwrap().into_inner()?
            .into_inner().map_err(|e| e.into_error())?.finish()?
            .sync_all()?;

        let temp_path = self.temp_path.clone().unwrap();
        let parent_path = temp_path.parent().unwrap();

        util::sys::fsync_directory(&temp_path)?;
        fs::rename(&temp_path, &self.path)?;
        self.temp_path = None;
        util::sys::fsync_directory(parent_path)?;

        Ok(())
    }

    fn deduplicate(
        &mut self, path: &Path, file: &mut File, fingerprint: &Fingerprint, size: u64,
    ) -> GenericResult<Option<(Hash, u64)>> {
        if size == 0 {
            debug!("{:?} has zero size.", path);
            return Ok(Some((EMPTY_FILE_HASH.clone(), size)))
        }

        if let Some(last_state) = self.last_state.as_ref().and_then(|states| states.get(path)) {
            if *fingerprint == last_state.fingerprint {
                debug!("{:?} hasn't been changed.", path);
                return Ok(Some((last_state.hash.clone(), size)));
            }
        }

        let mut file_reader = FileReader::new(file, size);
        io::copy(&mut file_reader, &mut io::sink())?;
        let (bytes_read, hash) = file_reader.consume();
        file.seek(SeekFrom::Start(0))?;

        if self.extern_hashes.contains(&hash) {
            debug!("Deduplicate {:?} by its hash.", path);
            return Ok(Some((hash, bytes_read)))
        }

        Ok(None)
    }

    fn data(&mut self) -> &mut Archive {
        self.data.as_mut().unwrap()
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

struct FileState {
    fingerprint: Fingerprint,
    hash: Hash,
}

fn load_backups_metadata(storage: &Storage, group: &BackupGroup) -> (
    HashSet<Hash>, Option<HashMap<PathBuf, FileState>>, bool,
) {
    let backups = &group.backups;
    let results = backups.par_iter().enumerate().map(|(index, backup): (usize, &Backup)| {
        let mut hashes = HashSet::new();
        let mut last_state = if index == backups.len() - 1 {
            Some(HashMap::new())
        } else {
            None
        };

        for file in backup.read_metadata(storage.provider.read())? {
            let file = file?;

            if let Some(last_state) = last_state.as_mut() {
                last_state.insert(file.path.into(), FileState {
                    fingerprint: file.fingerprint,
                    hash: file.hash.clone(),
                });
            }

            if file.unique {
                hashes.insert(file.hash);
            }
        }

        Ok((hashes, last_state))
    });

    let mut ok = true;
    let mut all_hashes = HashSet::new();
    let mut files_last_state = None;

    for (index, result) in results.collect::<Vec<GenericResult<_>>>().into_iter().enumerate() {
        match result {
            Ok((hashes, last_state)) => {
                all_hashes.extend(hashes);
                files_last_state = last_state;
            },
            Err(e) => {
                let backup = &backups[index];
                error!("Failed to load {:?} backup metadata: {}", backup.name, e);
                ok = false;
            },
        }
    }

    (all_hashes, files_last_state, ok)
}