use std::fs;
use std::io;

use core::{EmptyResult, GenericResult};
use provider::{ReadProvider, File, FileType};

pub struct Filesystem {
    path: String,
}

impl Filesystem {
    pub fn new(path: &str) -> Filesystem {
        Filesystem{path: path.to_owned()}
    }
}

impl ReadProvider for Filesystem {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>> {
        let entries = fs::read_dir(&self.path);

        if let Err(ref err) = entries {
            if err.kind() == io::ErrorKind::NotFound {
                return Ok(None);
            }
        }

        let mut files = Vec::new();

        for entry in entries? {
            let entry = entry?;

            let file_name = entry.file_name().into_string().map_err(|file_name| format!(
                "Got an invalid file name: {:?}", file_name.to_string_lossy()))?;

            let metadata = entry.metadata().map_err(|e| format!(
                "Unable to get metadata of {:?}: {}", entry.path().to_string_lossy(), e))?;

            files.push(File {
                name: file_name,
                type_: if metadata.is_file() {
                    FileType::File
                } else if metadata.is_dir() {
                    FileType::Directory
                } else {
                    FileType::Other
                },
            })
        }

        Ok(Some(files))
    }
}