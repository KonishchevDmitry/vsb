use std::io::Write;

use tar;

use encryptor::Encryptor;
use storage::Storage;

// FIXME
pub fn sync_backups(local_storage: &Storage, cloud_storage: &mut Storage) {
    local_storage.get_backup_groups().unwrap();

    if false {
        info!("> {:?}", local_storage.get_backup_groups().unwrap());
        let (encryptor, chunks) = Encryptor::new().unwrap();
        drop(chunks);

        let mut archive = tar::Builder::new(encryptor);
        archive.append_dir_all("backup", "backup-mock").unwrap();

        let mut encryptor = archive.into_inner().unwrap();
        encryptor.finish().map_err(|e| format!("Got an error: {}", e)).unwrap();
    }
}