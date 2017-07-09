use std::io::Write;

use tar;

use encryptor::Encryptor;
use provider::Provider;

pub struct Uploader {
    provider: Box<Provider>,
}

impl Uploader {
    pub fn new(provider: Box<Provider>) -> Uploader {
        Uploader {
            provider: provider,
        }
    }

    // FIXME
    pub fn test(&self) {
        let (encryptor, chunks) = Encryptor::new().unwrap();
//        drop(chunks);

        let mut archive = tar::Builder::new(encryptor);
        archive.append_dir_all("backup", "backup-mock").unwrap();

        let mut encryptor = archive.into_inner().unwrap();
        encryptor.finish().map_err(|e| format!("Got an error: {}", e)).unwrap();
    }
}