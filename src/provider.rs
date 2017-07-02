use core::GenericResult;

pub trait Provider {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>>;

    // FIXME
    fn test(&self) {
        info!(">>> {:?}", Provider::list_directory(self, ""))
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