use std::time::Duration;

use futures::Stream;
use hyper::Body;
use hyper::header::{Header, ContentLength, ContentType};
use log::LogLevel;
use serde::{ser, de};
use serde_json;
use serde_urlencoded;
use tokio_core::reactor::Timeout;

use core::GenericResult;
use super::{Method, Headers, StatusCode, Response, ResponseReader, RawResponseReader,
            JsonReplyReader, JsonErrorReader, HttpClientError};

// FIXME: pub?
// FIXME: lifetimes
pub struct Request<'a, R, E> {
    pub method: Method,
    pub url: String,
    pub headers: Headers,
    pub body: Option<Body>,
    pub timeout: Duration, // FIXME: default timeout / default headers?

    pub trace_headers: Vec<String>,
    pub trace_body: Option<String>,

    // FIXME: private
    pub reply_reader: Box<ResponseReader<Result=R> + 'a>,
    pub error_reader: Box<ResponseReader<Result=E> + 'a>,
}

impl<'a, R, E> Request<'a, R, E> {
    pub fn new<RR, ER>(method: Method, url: String, timeout: Duration,
                       reply_reader: RR, error_reader: ER) -> Request<'a, R, E>
        where RR: ResponseReader<Result=R> + 'a,
              ER: ResponseReader<Result=E> + 'a
    {
        Request {
            method: method,
            url: url.to_owned(),
            headers: Headers::new(),
            body: None,
            timeout: timeout,

            // FIXME
            trace_headers: Vec::new(),
            trace_body: None,

            reply_reader: Box::new(reply_reader),
            error_reader: Box::new(error_reader),
        }
    }

    pub fn with_params<P: ser::Serialize>(mut self, params: &P) -> GenericResult<Request<'a, R, E>> {
        let query_string = serde_urlencoded::to_string(params)?;

        self.url += if self.url.contains('?') {
            "&"
        } else {
            "?"
        };

        self.url += &query_string;

        Ok(self)
    }

    // FIXME: ::std::fmt::Display
    pub fn with_header<H: Header + ::std::fmt::Display>(mut self, header: H, trace: bool) -> Request<'a, R, E> {
        if trace {
            // FIXME
            self.trace_headers.push(header.to_string())
        }
        self.headers.set(header);
        self
    }

    pub fn with_body<B: Into<Body>>(mut self, content_type: ContentType, content_length: Option<u64>,
                                    body: B) -> GenericResult<Request<'a, R, E>> {
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

    pub fn with_text_body(mut self, content_type: ContentType, body: String) -> GenericResult<Request<'a, R, E>> {
        let content_length = Some(body.len() as u64);

        if log_enabled!(LogLevel::Trace) {
            let mut request = self.with_body(content_type, content_length, body.clone())?;
            request.trace_body = Some(body);
            return Ok(request);
        } else {
            return Ok(self.with_body(content_type, content_length, body)?);
        }
    }

    pub fn with_form<B: ser::Serialize>(mut self, request: &B) -> GenericResult<Request<'a, R, E>> {
        let body = serde_urlencoded::to_string(request)?;
        Ok(self.with_text_body(ContentType::form_url_encoded(), body)?)
    }

    pub fn with_json<B: ser::Serialize>(mut self, request: &B) -> GenericResult<Request<'a, R, E>> {
        let body = serde_json::to_string(request)?;
        Ok(self.with_text_body(ContentType::json(), body)?)
    }
}

impl<'a, R: de::DeserializeOwned + 'a, E: de::DeserializeOwned + 'a> Request<'a, R, E> {
    pub fn new_json(method: Method, url: String, timeout: Duration) -> Request<'a, R, E> {
        Request::new(method, url, timeout, JsonReplyReader::new(), JsonErrorReader::new())
    }
}