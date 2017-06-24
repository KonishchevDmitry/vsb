pub type EmptyResult = GenericResult<()>;
pub type GenericError = Box<::std::error::Error + Send + Sync>;
pub type GenericResult<T> = Result<T, GenericError>;