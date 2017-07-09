use std::io::Write;

use tar;

use encryptor::Encryptor;
use provider::{ReadProvider, WriteProvider};
use storage::Storage;

pub struct Uploader {
    local_storage: Storage,
    cloud_storage: Storage,
}

impl Uploader {
    pub fn new(local_storage: Storage, cloud_storage: Storage) -> Uploader {
        Uploader {
            local_storage: local_storage,
            cloud_storage: cloud_storage,
        }
    }

    // FIXME
    pub fn test(&self) {
        self.local_storage.get_backup_groups();
//        let local_storage = BackupStorage::new("/Users/konishchev/.backup");
//        info!("> {:?}", local_storage.get_backup_groups().unwrap());
//        let (encryptor, chunks) = Encryptor::new().unwrap();
////        drop(chunks);
//
//        let mut archive = tar::Builder::new(encryptor);
//        archive.append_dir_all("backup", "backup-mock").unwrap();
//
//        let mut encryptor = archive.into_inner().unwrap();
//        encryptor.finish().map_err(|e| format!("Got an error: {}", e)).unwrap();
    }
}