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
        let encryptor = Encryptor::new().unwrap();
        let mut a = tar::Builder::new(encryptor);

        a.append_dir_all("backup", "backup-mock").unwrap();
        a.finish().unwrap()
    }
}