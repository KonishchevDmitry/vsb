use std::borrow::Cow;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use hyper::Body;
use hyper::header::{Header, Raw, ContentLength, ContentType};
use log::LogLevel;
use serde::{ser, de};
use serde_json;
use serde_urlencoded;

use super::{Method, Headers, ResponseReader, JsonReplyReader, JsonErrorReader};

// FIXME: pub?
// FIXME: lifetimes
pub struct HttpRequest<'a, R, E> {
    pub method: Method,
    pub url: String,
    pub headers: Headers,
    pub body: Option<Body>,
    pub timeout: Duration, // FIXME: default timeout / default headers?

    pub trace_body: Option<String>,

    // FIXME: private
    pub reply_reader: Box<ResponseReader<Result=R> + 'a>,
    pub error_reader: Box<ResponseReader<Result=E> + 'a>,
}

pub type HttpRequestBuildingResult<'a, R, E> = Result<HttpRequest<'a, R, E>, HttpRequestBuildingError>;

impl<'a, R, E> HttpRequest<'a, R, E> {
    pub fn new<RR, ER>(method: Method, url: String, timeout: Duration,
                       reply_reader: RR, error_reader: ER) -> HttpRequest<'a, R, E>
        where RR: ResponseReader<Result=R> + 'a,
              ER: ResponseReader<Result=E> + 'a
    {
        HttpRequest {
            method: method,
            url: url.to_owned(),
            headers: Headers::new(),
            body: None,
            timeout: timeout,

            trace_body: None,

            reply_reader: Box::new(reply_reader),
            error_reader: Box::new(error_reader),
        }
    }

    pub fn with_params<P: ser::Serialize>(mut self, params: &P) -> HttpRequestBuildingResult<'a, R, E> {
        let query_string = serde_urlencoded::to_string(params)
            .map_err(HttpRequestBuildingError::new)?;

        self.url += if self.url.contains('?') {
            "&"
        } else {
            "?"
        };

        self.url += &query_string;

        Ok(self)
    }

    pub fn with_header<H: Header>(mut self, header: H) -> HttpRequest<'a, R, E> {
        self.headers.set(header);
        self
    }

    pub fn with_raw_header<K: Into<Cow<'static, str>>, V: Into<Raw>>(mut self, name: K, value: V) -> HttpRequest<'a, R, E> {
        self.headers.set_raw(name, value);
        self
    }

    pub fn with_body<B: Into<Body>>(mut self, content_type: ContentType, content_length: Option<u64>,
                                    body: B) -> HttpRequestBuildingResult<'a, R, E> {
        // FIXME: panic?
        if self.body.is_some() {
            return Err(HttpRequestBuildingError::new("An attempt to set request body twice"))
        }

        self.headers.set(content_type);
        if let Some(content_length) = content_length {
            self.headers.set(ContentLength(content_length));
        }

        self.body = Some(body.into());

        Ok(self)
    }

    pub fn with_text_body<B: Into<String>>(self, content_type: ContentType, body: B) -> HttpRequestBuildingResult<'a, R, E> {
        let body = body.into();
        let content_length = Some(body.len() as u64);

        if log_enabled!(LogLevel::Trace) {
            let mut request = self.with_body(content_type, content_length, body.clone())?;
            request.trace_body = Some(body);
            return Ok(request);
        } else {
            return Ok(self.with_body(content_type, content_length, body)?);
        }
    }

    pub fn with_form<B: ser::Serialize>(self, request: &B) -> HttpRequestBuildingResult<'a, R, E> {
        let body = serde_urlencoded::to_string(request).map_err(HttpRequestBuildingError::new)?;
        Ok(self.with_text_body(ContentType::form_url_encoded(), body)?)
    }

    pub fn with_json<B: ser::Serialize>(self, request: &B) -> HttpRequestBuildingResult<'a, R, E> {
        let body = serde_json::to_string(request).map_err(HttpRequestBuildingError::new)?;
        Ok(self.with_text_body(ContentType::json(), body)?)
    }
}

impl<'a, R: de::DeserializeOwned + 'a, E: de::DeserializeOwned + 'a> HttpRequest<'a, R, E> {
    pub fn new_json(method: Method, url: String, timeout: Duration) -> HttpRequest<'a, R, E> {
        HttpRequest::new(method, url, timeout, JsonReplyReader::new(), JsonErrorReader::new())
    }
}

#[derive(Debug)]
pub struct HttpRequestBuildingError(String);

impl HttpRequestBuildingError {
    pub fn new<E: ToString>(err: E) -> HttpRequestBuildingError {
        HttpRequestBuildingError(err.to_string())
    }
}

impl Error for HttpRequestBuildingError {
    fn description(&self) -> &str {
        "HTTP request building error"
    }
}

impl fmt::Display for HttpRequestBuildingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.description(), self.0)
    }
}

#[derive(Debug, Serialize)]
pub struct EmptyRequest {
}