use serde::de;
use serde_json;

use core::GenericResult;

pub trait ResponseReader {
    type Result;

    fn read(&self, body: String) -> GenericResult<Self::Result>;
}

pub struct JsonResponseReader<T> {
    hack: Option<T>
}

impl<T: de::DeserializeOwned> ResponseReader for JsonResponseReader<T> {
    type Result = T;

    fn read(&self, body: String) -> GenericResult<Self::Result> {
        Ok(serde_json::from_str(&body)?)
    }
}