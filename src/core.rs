pub type EmptyResult = GenericResult<()>;
pub type GenericError = Box<::std::error::Error + Send + Sync>;
pub type GenericResult<T> = Result<T, GenericError>;

// FIXME
macro_rules! format_to {
    ($($arg:tt)*) => (::std::convert::From::from(format!($($arg)*)))
}

// FIXME
macro_rules! Err {
    ($($arg:tt)*) => (::std::result::Result::Err(format_to!($($arg)*)))
}
