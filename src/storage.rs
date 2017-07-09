use core::GenericResult;
use provider::{ReadProvider, WriteProvider};

#[derive(Debug)]
pub struct BackupGroup {
    pub backups: Vec<String>,
}

pub struct Storage {
    provider: Box<AbstractProvider>,
}

impl Storage {
    pub fn new<T: ReadProvider + WriteProvider + 'static>(provider: T) -> Storage {
        Storage {
            provider: Box::new(ReadWriteProviderAdapter{provider: provider}),
        }
    }

    pub fn new_read_only<T: ReadProvider +'static>(provider: T) -> Storage {
        Storage {
            provider: Box::new(ReadOnlyProviderAdapter{provider: provider}),
        }
    }

    // FIXME
    pub fn get_backup_groups(&self) -> GenericResult<Vec<BackupGroup>> {
        self.provider.read().list_directory("fsf")?;
        panic!("FIXME")
    }
}

// FIXME: Rust don't have trait upcasting yet (https://github.com/rust-lang/rust/issues/5665), so we
// have to emulate it via this trait.
trait AbstractProvider {
    fn read(&self) -> &ReadProvider;
    fn write(&mut self) -> GenericResult<&WriteProvider>;
}

struct ReadOnlyProviderAdapter<T: ReadProvider> {
    provider: T,
}

impl<T: ReadProvider> AbstractProvider for ReadOnlyProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&mut self) -> GenericResult<&WriteProvider> {
        Err!("An attempt to modify a read-only backup storage")
    }
}

struct ReadWriteProviderAdapter<T: ReadProvider + WriteProvider> {
    provider: T,
}

impl<T: ReadProvider + WriteProvider> AbstractProvider for ReadWriteProviderAdapter<T> {
    fn read(&self) -> &ReadProvider {
        &self.provider
    }

    fn write(&mut self) -> GenericResult<&WriteProvider> {
        Ok(&self.provider)
    }
}