use super::{StatusCode, Headers};

pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: Headers,
    pub body: Vec<u8>,
}