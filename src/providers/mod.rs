pub mod dropbox;
pub mod filesystem;
pub mod google_drive;
pub mod yandex_disk;

mod oauth;

use std::fmt;
use std::io;

use crate::core::{EmptyResult, GenericResult};
use crate::util::hash::Hasher;
use crate::util::stream_splitter::ChunkStreamReceiver;

pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn type_(&self) -> ProviderType;

    fn clarification(&self) -> String {
        match self.type_() {
            ProviderType::Cloud => format!(" on {}", self.name()),
            ProviderType::Local => String::new(),
        }
    }
}

pub trait ReadProvider: Provider {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>>;

    fn open_file(&self, _path: &str) -> GenericResult<Box<dyn io::Read>> {
        Err!("{} provider doesn't support file opening functionality", self.name())
    }
}

pub trait WriteProvider: Provider {
    fn create_directory(&self, path: &str) -> EmptyResult;
    fn delete(&self, path: &str) -> EmptyResult;
}

pub trait UploadProvider: Provider {
    fn hasher(&self) -> Box<dyn Hasher>;
    fn max_request_size(&self) -> Option<u64>;
    fn upload_file(&self, directory_path: &str, temp_name: &str, name: &str,
                   chunk_streams: ChunkStreamReceiver) -> EmptyResult;
}

pub enum ProviderType {
    Local,
    Cloud,
}

#[derive(Debug)]
pub struct File {
    pub name: String,
    pub type_: FileType,
    pub size: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum FileType {
    File,
    Directory,
    Other,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            FileType::Directory => "directory",
            FileType::File | FileType::Other => "file",
        })
    }
}
