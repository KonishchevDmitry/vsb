// FIXME

pub mod client;
pub mod response;
pub mod request;

mod readers;

pub use hyper::StatusCode;
pub use self::client::{HttpClient, Method, Headers, EmptyResponse, HttpClientError};
pub use self::request::HttpRequest;
pub use self::response::HttpResponse;
pub use self::readers::*;