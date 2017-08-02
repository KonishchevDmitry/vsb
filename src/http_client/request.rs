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
use super::{Method, Headers, StatusCode, Response, ResponseReader, JsonReplyReader, JsonErrorReader,
            HttpClientError};

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


// FIXME
pub struct NewRequest<R, E> {
    reply_reader: Box<ResponseReader<Result=R>>,
    error_reader: Box<ResponseReader<Result=E>>,
}

impl<R, E> NewRequest<R, E> {
    fn new<RR, ER>(reply_reader: RR, error_reader: ER) -> NewRequest<R, E>
        where RR: ResponseReader<Result=R> + 'static,
              ER: ResponseReader<Result=E> + 'static
    {
        NewRequest {
            reply_reader: Box::new(reply_reader),
            error_reader: Box::new(error_reader),
        }
    }

    fn get_result(&self, response: Response) -> Result<R, HttpClientError<E>> {
        if response.status.is_success() {
            Ok(self.reply_reader.read(response).map_err(HttpClientError::generic_from)?)
        } else if response.status.is_client_error() || response.status.is_server_error() {
            Err(HttpClientError::Api(
                self.error_reader.read(response).map_err(HttpClientError::generic_from)?))
        } else {
            Err!("Server returned an error: {}", response.status)
        }
    }
}

impl<R: de::DeserializeOwned + 'static, E: de::DeserializeOwned + 'static> NewRequest<R, E> {
    fn new_json() -> NewRequest<R, E> {
        NewRequest::new(JsonReplyReader::new(), JsonErrorReader::new())
    }
}