pub type EmptyResult = GenericResult<()>;
pub type GenericResult<T> = Result<T, GenericError>;
pub type GenericError = Box<dyn std::error::Error + Send + Sync>;

// FIXME(konishchev): Drop?
#[cfg(test)]
macro_rules! s {
    ($e:expr) => ($e.to_owned())
}

macro_rules! Err {
    ($($arg:tt)*) => (::std::result::Result::Err(format!($($arg)*).into()))
}
