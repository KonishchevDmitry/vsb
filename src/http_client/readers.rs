use std::marker::PhantomData;

use hyper::header::ContentType;
use hyper::mime;
use serde::de;
use serde_json;

use core::GenericResult;
use super::response::HttpResponse;

pub trait ResponseReader {
    type Result;

    fn read(&self, response: HttpResponse) -> GenericResult<Self::Result>;
}

pub struct JsonReplyReader<T> {
    phantom: PhantomData<T>,
}

impl<T: de::DeserializeOwned> JsonReplyReader<T> {
    pub fn new() -> JsonReplyReader<T> {
        JsonReplyReader{
            phantom: PhantomData
        }
    }
}

impl<T: de::DeserializeOwned> ResponseReader for JsonReplyReader<T> {
    type Result = T;

    fn read(&self, response: HttpResponse) -> GenericResult<Self::Result> {
        let content_type = response.headers.get::<ContentType>().ok_or_else(|| format!(
            "Server returned {} response without Content-Type", response.status))?;

        if content_type.type_() != mime::APPLICATION || content_type.subtype() != mime::JSON {
            return Err!("Server returned {} response with an invalid content type: {}",
                        response.status, content_type)
        }

        Ok(serde_json::from_slice(&response.body).map_err(|e| format!(
            "Server returned an invalid JSON response: {}", e))?)
    }
}

pub struct JsonErrorReader<T> {
    phantom: PhantomData<T>,
}

impl<T: de::DeserializeOwned> JsonErrorReader<T> {
    pub fn new() -> JsonErrorReader<T> {
        JsonErrorReader{
            phantom: PhantomData
        }
    }

    fn read_plain_text_error(&self, response: HttpResponse) -> String {
        if let Ok(body) = String::from_utf8(response.body) {
            let error = body.lines().next().unwrap_or("").trim_right_matches('.').trim();
            if !error.is_empty() {
                return error.to_owned()
            }
        }

        return response.status.to_string();
    }
}

impl<T: de::DeserializeOwned> ResponseReader for JsonErrorReader<T> {
    type Result = T;

    fn read(&self, response: HttpResponse) -> GenericResult<Self::Result> {
        let content_type = response.headers.get::<ContentType>().map(Clone::clone).ok_or_else(|| format!(
            "Server returned {} error without Content-Type", response.status))?;

        if content_type.type_() == mime::APPLICATION && content_type.subtype() == mime::JSON {
            Ok(serde_json::from_slice(&response.body).map_err(|e| format!(
                "Server returned an invalid JSON response: {}", e))?)
        } else if content_type.type_() == mime::TEXT && content_type.subtype() == mime::PLAIN {
            Err!("Server returned an error: {}", self.read_plain_text_error(response))
        } else {
            Err!("Server returned {} error with an invalid content type: {}",
                 response.status, content_type)
        }
    }
}

// FIXME
//pub struct HttpStatusReader {
//}
//
//impl HttpStatusReader {
//    pub fn new() -> HttpStatusReader {
//        HttpStatusReader {}
//    }
//}
//
//impl ResponseReader for HttpStatusReader {
//    type Result = String;
//
//    fn read(&self, response: HttpResponse) -> GenericResult<Self::Result> {
//        Ok(response.status.to_string())
//    }
//}

pub struct RawResponseReader {
}

impl RawResponseReader {
    pub fn new() -> RawResponseReader {
        RawResponseReader {}
    }
}

impl ResponseReader for RawResponseReader {
    type Result = HttpResponse;

    fn read(&self, response: HttpResponse) -> GenericResult<Self::Result> {
        Ok(response)
    }
}