use core::{GenericResult, EmptyResult};

#[derive(Debug)]
pub struct File {
    pub name: String,
    pub type_: FileType,
}

#[derive(Debug)]
pub enum FileType {
    File,
    Directory,
    Other,
}

pub trait ReadProvider {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>>;
}

pub trait WriteProvider {
    fn upload_file(&self, path: &str) -> EmptyResult;
}