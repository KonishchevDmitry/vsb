pub type EmptyResult = GenericResult<()>;
pub type GenericError = Box<::std::error::Error + Send + Sync>;
pub type GenericResult<T> = Result<T, GenericError>;

macro_rules! Err {
    ($($arg:tt)*) => (::std::result::Result::Err(format!($($arg)*).into()))
}
