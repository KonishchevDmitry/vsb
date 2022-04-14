use std::fs;
use std::io;

use crate::core::{EmptyResult, GenericResult};
use crate::util::hash::Hasher;
use crate::util::stream_splitter::ChunkStreamReceiver;

use super::{Provider, ProviderType, ReadProvider, WriteProvider, File, FileType};

pub struct Filesystem {
}

impl Filesystem {
    pub fn new() -> Filesystem {
        Filesystem{}
    }
}

impl Provider for Filesystem {
    fn name(&self) -> &'static str {
        "Local storage"
    }

    fn type_(&self) -> ProviderType {
        ProviderType::Local
    }
}

impl ReadProvider for Filesystem {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>> {
        let entries = fs::read_dir(path);

        if let Err(ref err) = entries {
            if err.kind() == io::ErrorKind::NotFound {
                return Ok(None);
            }
        }

        let mut files = Vec::new();

        for entry in entries? {
            let entry = entry?;

            let name = entry.file_name().into_string().map_err(|file_name| format!(
                "Got an invalid file name: {:?}", file_name.to_string_lossy()))?;

            let metadata = entry.metadata().map_err(|e| format!(
                "Unable to get metadata of {:?}: {}", entry.path().to_string_lossy(), e))?;

            let type_ = if metadata.is_file() {
                FileType::File
            } else if metadata.is_dir() {
                FileType::Directory
            } else {
                FileType::Other
            };

            let size = match type_ {
                FileType::File => Some(metadata.len()),
                FileType::Directory | FileType::Other => None,
            };

            files.push(File {name, type_, size})
        }

        Ok(Some(files))
    }

    fn open_file(&self, path: &str) -> GenericResult<Box<dyn io::Read>> {
        Ok(Box::new(fs::File::open(path)?))
    }
}

impl WriteProvider for Filesystem {
    // FIXME(konishchev): Implement
    fn hasher(&self) -> Box<dyn Hasher> {
        unreachable!()
    }

    // FIXME(konishchev): Implement
    fn max_request_size(&self) -> Option<u64> {
        unreachable!()
    }

    // FIXME(konishchev): Implement
    fn create_directory(&self, path: &str) -> EmptyResult {
        Ok(fs::create_dir(path)?)
    }

    // FIXME(konishchev): Implement
    fn upload_file(&self, _directory_path: &str, _temp_name: &str, _name: &str,
                   _chunk_streams: ChunkStreamReceiver) -> EmptyResult {
        unreachable!()
    }

    // FIXME(konishchev): Implement
    fn delete(&self, _path: &str) -> EmptyResult {
        unreachable!()
    }
}