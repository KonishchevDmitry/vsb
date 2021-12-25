use serde_derive::Deserialize;

use crate::core::GenericResult;

use super::{StatusCode, Headers, HeaderName};

pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: Headers,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn get_header(&self, name: HeaderName) -> GenericResult<Option<&str>> {
        let value = match self.headers.get(&name) {
            Some(value) => value,
            None => return Ok(None),
        };

        Ok(Some(value.to_str().map_err(|_| format!(
            "Got invalid {:?} header value: {:?}", name, value))?))
    }
}

#[derive(Debug, Deserialize)]
pub struct EmptyResponse {
}