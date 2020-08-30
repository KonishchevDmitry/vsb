use crate::core::GenericResult;
use crate::provider::{ReadProvider, WriteProvider};

// Rust don't have trait upcasting yet (https://github.com/rust-lang/rust/issues/5665), so we have
// to emulate it via this trait.
pub trait AbstractProvider {
    fn read(&self) -> &dyn ReadProvider;
    fn write(&self) -> GenericResult<&dyn WriteProvider>;
}

pub struct ReadOnlyProviderAdapter<T: ReadProvider> {
    provider: T,
}

impl<T: ReadProvider + 'static> ReadOnlyProviderAdapter<T> {
    pub fn new(provider: T) -> Box<dyn AbstractProvider> {
        Box::new(ReadOnlyProviderAdapter{provider})
    }
}

impl<T: ReadProvider> AbstractProvider for ReadOnlyProviderAdapter<T> {
    fn read(&self) -> &dyn ReadProvider {
        &self.provider
    }

    fn write(&self) -> GenericResult<&dyn WriteProvider> {
        Err!("An attempt to modify a read-only backup storage")
    }
}

pub struct ReadWriteProviderAdapter<T: ReadProvider + WriteProvider> {
    provider: T,
}

impl<T: ReadProvider + WriteProvider + 'static> ReadWriteProviderAdapter<T> {
    pub fn new(provider: T) -> Box<dyn AbstractProvider> {
        Box::new(ReadWriteProviderAdapter{provider})
    }
}

impl<T: ReadProvider + WriteProvider> AbstractProvider for ReadWriteProviderAdapter<T> {
    fn read(&self) -> &dyn ReadProvider {
        &self.provider
    }

    fn write(&self) -> GenericResult<&dyn WriteProvider> {
        Ok(&self.provider)
    }
}