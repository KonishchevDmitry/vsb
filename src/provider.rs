use std::fmt;

use futures::sync::mpsc;
use hyper::{self, Chunk};

use core::{GenericResult, EmptyResult};

pub trait Provider {
    fn name(&self) -> &'static str;
    fn type_(&self) -> ProviderType;
}

pub trait ReadProvider: Provider {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>>;
}

pub trait WriteProvider: Provider {
    fn create_directory(&self, path: &str) -> EmptyResult;
    fn upload_file(&self, path: &str, data: ChunkReceiver) -> EmptyResult;
}

pub enum ProviderType {
    Local,
    Cloud,
}

#[derive(Debug)]
pub struct File {
    pub name: String,
    pub type_: FileType,
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

pub type ChunkReceiver = mpsc::Receiver<ChunkResult>;
pub type ChunkResult = Result<Chunk, hyper::Error>;