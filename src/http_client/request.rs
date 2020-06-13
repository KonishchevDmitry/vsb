use std::error::Error;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use log;
use serde::{ser, de};
use serde_json;
use serde_urlencoded;

use super::{Method, Headers, HeaderName, Body, ResponseReader, JsonReplyReader, JsonErrorReader,
            headers};

pub struct HttpRequest<'a, R, E> {
    pub method: Method,
    pub url: String,
    pub headers: Headers,
    pub timeout: Duration,

    pub body: Option<Body>,
    pub trace_body: Option<String>,

    pub reply_reader: Box<dyn ResponseReader<Result=R> + 'a>,
    pub error_reader: Box<dyn ResponseReader<Result=E> + 'a>,
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
            url: url,
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

    pub fn with_header<K: AsRef<str>, V:AsRef<str>>(mut self, name: K, value: V) -> HttpRequestBuildingResult<'a, R, E> {
        let name = HeaderName::from_str(name.as_ref()).map_err(|_| HttpRequestBuildingError::new(format!(
            "Invalid header name: {:?}", name.as_ref())))?;

        let value = value.as_ref().parse().map_err(|_| HttpRequestBuildingError::new(format!(
            "Invalid {:?} header value", name.as_str())))?;

        self.headers.insert(name, value);
        Ok(self)
    }

    pub fn with_body<B: Into<Body>>(mut self, content_type: &str, body: B) -> HttpRequestBuildingResult<'a, R, E> {
        if self.body.is_some() {
            return Err(HttpRequestBuildingError::new("An attempt to set request body twice"))
        }

        self.body = Some(body.into());
        Ok(self.with_header(headers::CONTENT_TYPE, content_type)?)
    }

    pub fn with_text_body<B: Into<String>>(self, content_type: &str, data: B) -> HttpRequestBuildingResult<'a, R, E> {
        let body = data.into();

        Ok(if log_enabled!(log::Level::Trace) {
            let mut request = self.with_body(content_type, body.clone())?;
            request.trace_body = Some(body);
            request
        } else {
            self.with_body(content_type, body)?
        })
    }

    pub fn with_form<B: ser::Serialize>(self, request: &B) -> HttpRequestBuildingResult<'a, R, E> {
        let body = serde_urlencoded::to_string(request).map_err(HttpRequestBuildingError::new)?;
        Ok(self.with_text_body("application/x-www-form-urlencoded", body)?)
    }

    pub fn with_json<B: ser::Serialize>(self, request: &B) -> HttpRequestBuildingResult<'a, R, E> {
        let body = serde_json::to_string(request).map_err(HttpRequestBuildingError::new)?;
        Ok(self.with_text_body("application/json", body)?)
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
}

impl fmt::Display for HttpRequestBuildingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "HTTP request building error: {}", self.0)
    }
}

#[derive(Debug, Serialize)]
pub struct EmptyRequest {
}