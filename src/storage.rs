use core::GenericResult;
use provider::{ReadProvider, WriteProvider};

#[derive(Debug)]
pub struct BackupGroup {
    pub backups: Vec<String>,
}

struct ReadOnlyProviderAdapter<T: ReadProvider> {
    provider: T,
}

impl<T: ReadProvider> AbstractProvider for ReadOnlyProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&mut self) -> &WriteProvider {
        panic!(1)
    }
}

struct ReadWriteProviderAdapter<T: ReadProvider + WriteProvider> {
    provider: T,
}

impl<T: ReadProvider + WriteProvider> AbstractProvider for ReadWriteProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&mut self) -> &WriteProvider {
        &self.provider
    }
}

trait AbstractProvider {
    fn read(&self) -> &ReadProvider;
    fn write(&mut self) -> &WriteProvider;
}

pub struct Storage {
    provider: Box<AbstractProvider>,
}

// FIXME: Trait upcasting https://github.com/rust-lang/rust/issues/5665

impl Storage {
    pub fn new<T: ReadProvider + WriteProvider + 'static>(provider: T) -> Storage {
        let adapter = ReadWriteProviderAdapter{provider: provider};
        let test: Box<AbstractProvider> = Box::new(adapter);
        Storage {
            provider: test,
        }
    }

    pub fn new_read_only<T: ReadProvider +'static>(provider: T) -> Storage {
        let adapter = ReadOnlyProviderAdapter{provider: provider};
        let test: Box<AbstractProvider> = Box::new(adapter);
        Storage {
            provider: test,
        }
    }

    pub fn get_backup_groups(&self) -> GenericResult<Vec<BackupGroup>> {
        self.provider.read().list_directory("fsf");
//        self.provider.write().upload_file("fsf");
        panic!("dfsf")
    }
}