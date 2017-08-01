use std::error::Error;
use std::fmt;
use std::io;
use std::time::{Instant, Duration};

use futures::{Future, Stream};
use hyper::{self, Client, Headers, Body, Response, StatusCode, Chunk};
use hyper::header::{Header, UserAgent, ContentLength, ContentType};
use hyper_tls::HttpsConnector;
use log::LogLevel;
use mime;
use serde::{ser, de};
use serde_json;
use serde_urlencoded;
use tokio_core::reactor::{Core, Timeout};

pub use hyper::Method;

use core::GenericResult;

// FIXME
//pub trait Iterator {
//    type Item;
//    fn next(&mut self) -> Option<Self::Item>;
//}
//
//impl Iterator for Counter {
//    type Item = u32;
//
//    fn next(&mut self) -> Option<Self::Item> {}
//    }
//}

// FIXME: pub?
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Headers,
    pub body: Option<Body>,
    pub timeout: Duration,

    pub trace_body: Option<String>,
}

impl Request {
    pub fn new(method: Method, url: String, timeout: Duration) -> Request {
        Request {
            method: method,
            url: url.to_owned(),
            headers: Headers::new(),
            body: None,
            timeout: timeout,

            trace_body: None,
        }
    }

    pub fn with_params<P: ser::Serialize>(mut self, params: &P) -> GenericResult<Request> {
        let query_string = serde_urlencoded::to_string(params)?;

        self.url += if self.url.contains('?') {
            "&"
        } else {
            "?"
        };

        self.url += &query_string;

        Ok(self)
    }

    pub fn with_header<H: Header>(mut self, header: H) -> Request {
        self.headers.set(header);
        self
    }

    pub fn with_body<B: Into<Body>>(mut self, content_type: ContentType, content_length: Option<u64>,
                                    body: B) -> GenericResult<Request> {
        if self.body.is_some() {
            return Err!("An attempt to set request body twice")
        }

        self.headers.set(content_type);
        if let Some(content_length) = content_length {
            self.headers.set(ContentLength(content_length));
        }

        self.body = Some(body.into());

        Ok(self)
    }

    pub fn with_text_body(mut self, content_type: ContentType, body: String) -> GenericResult<Request> {
        let content_length = Some(body.len() as u64);

        if log_enabled!(LogLevel::Trace) {
            let mut request = self.with_body(content_type, content_length, body.clone())?;
            request.trace_body = Some(body);
            return Ok(request);
        } else {
            return Ok(self.with_body(content_type, content_length, body)?);
        }
    }

    pub fn with_form<R: ser::Serialize>(mut self, request: &R) -> GenericResult<Request> {
        let body = serde_urlencoded::to_string(request)?;
        Ok(self.with_text_body(ContentType::form_url_encoded(), body)?)
    }

    pub fn with_json<R: ser::Serialize>(mut self, request: &R) -> GenericResult<Request> {
        let body = serde_json::to_string(request)?;
        Ok(self.with_text_body(ContentType::json(), body)?)
    }
}