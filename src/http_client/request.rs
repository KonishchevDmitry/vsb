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

use super::HttpClientError;
use super::readers::{ResponseReader, JsonResponseReader};

// FIXME
struct Req<R, E> {
    reply_reader: Box<ResponseReader<Result=R>>,
    error_reader: Box<ResponseReader<Result=E>>,
}

impl<R, E> Req<R, E> {
    fn new<RR, ER>(reply_reader: RR, error_reader: ER) -> Req<R, E>
        where RR: ResponseReader<Result=R> + 'static,
              ER: ResponseReader<Result=E> + 'static
    {
        Req {
            reply_reader: Box::new(reply_reader),
            error_reader: Box::new(error_reader),
        }
    }

    fn get_response(&self, status: StatusCode, headers: &Headers, body: String) -> Result<R, HttpClientError<E>> {
        unimplemented!()
//        let content_type = headers.get::<ContentType>().map(Clone::clone);
//
//        if status == StatusCode::Ok {
//            return if content_type.type_() == mime::APPLICATION && content_type.subtype() == mime::JSON {
//                Ok(self.reply_reader.read(body).map_err(|e|
//                    format!("Got an invalid response from server: {}", e))?)
//            } else {
//                Err!("Server returned {} response with an invalid content type: {}",
//                    status, content_type)
//            }
//        } else {
//            return if status.is_client_error() || status.is_server_error() {
//                Err(HttpClientError::Api(parse_api_error(status, content_type, &body)
//                    .map_err(HttpClientError::generic_from)?))
//            } else {
//                Err!("Server returned an error: {}", status)
//            }
//        }
    }
}

//fn parse_api_error<T>(status: StatusCode, content_type: Option<ContentType>, body: &str) -> GenericResult<T>
//    where T: de::DeserializeOwned
//{
//    let content_type = content_type.ok_or_else(|| format!(
//        "Server returned {} error with an invalid content type", status))?;
//
//    if content_type.type_() == mime::TEXT && content_type.subtype() == mime::PLAIN {
//        let mut error = body.lines().next().unwrap_or("").trim_right_matches('.').trim().to_owned();
//        if error.is_empty() {
//            error = status.to_string();
//        }
//        return Err!("Server returned an error: {}", error);
//    } else if content_type.type_() == mime::APPLICATION && content_type.subtype() == mime::JSON {
//        return Ok(serde_json::from_str(body).map_err(|e| format!(
//            "Server returned an invalid JSON response: {}", e))?);
//    }
//
//    Err!("Server returned {} error with an invalid content type: {}",
//        status, content_type)
//}

//fn test() {
//    #[derive(Debug, Deserialize)]
//    struct Response {
//        has_more: bool,
//    }
//
//    let headers = Headers::new();
//
//    let request = Req::new(JsonResponseReader{hack: None});
//    let response: Response = request.get_response(StatusCode::Ok, &headers, "".to_owned());
//}