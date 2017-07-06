use core::{GenericResult, EmptyResult};

pub trait Provider {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>>;
    fn upload_file(&self, path: &str) -> EmptyResult;

    // FIXME
    fn test(&self) {
//        info!(">>> {:?}", Provider::list_directory(self, ""))
        info!(">>> {:?}", Provider::upload_file(self, ""))
    }
}

#[derive(Debug)]
pub struct File {
    pub name: String,
    pub type_: FileType,
}

#[derive(Debug)]
pub enum FileType {
    File,
    Directory,
}