mod body;
pub mod headers;
mod readers;
mod request;
mod response;

use std::error::Error;
use std::fmt;
use std::time::Duration;

use log::{log_enabled, trace};
use reqwest::blocking::Client;

use crate::core::GenericResult;

pub use reqwest::{Method, StatusCode};
pub use reqwest::header::{HeaderMap as Headers, HeaderName, HeaderValue};
pub use self::body::*;
pub use self::request::*;
pub use self::response::*;
pub use self::readers::*;

pub struct HttpClient {
    default_headers: Headers,
}

impl HttpClient {
    pub fn new() -> HttpClient {
        HttpClient {
            default_headers: Headers::new(),
        }.with_default_header(
            headers::USER_AGENT, "vsb (https://github.com/KonishchevDmitry/vsb)",
        ).unwrap()
    }

    pub fn with_default_header<V: AsRef<str>>(mut self, name: HeaderName, value: V) -> GenericResult<HttpClient> {
        let value = value.as_ref().parse().map_err(|_| format!(
            "Invalid {:?} header value", name.as_str()))?;
        self.default_headers.insert(name, value);
        Ok(self)
    }

    pub fn send<R, E>(&self, mut request: HttpRequest<R, E>) -> Result<R, HttpClientError<E>> {
        let mut headers = self.default_headers.clone();
        for (name, value) in request.headers.drain() {
            headers.insert(name.unwrap(), value);
        }

        if log_enabled!(log::Level::Trace) {
            let mut extra_info = String::new();

            if !headers.is_empty() {
                extra_info += "\n";
                extra_info += &headers.iter()
                    .map(|(name, value)| format!(
                        "{}: {}", name, value.to_str().unwrap_or("[non-ASCII data]")))
                    .collect::<Vec<_>>().join("\n");
            }

            if let Some(body) = request.trace_body {
                extra_info += "\n";
                extra_info += &body;
            }

            if extra_info.is_empty() {
                extra_info += "...";
            } else {
                extra_info.insert(0, ':');
            }

            trace!("Sending {method} {url}{extra_info}",
                   method=request.method, url=request.url, extra_info=extra_info);
        }

        let response = self.send_request(
            request.method, &request.url, headers, request.body, request.timeout)?;

        if response.status.is_success() {
            Ok(request.reply_reader.read(response)?)
        } else if response.status.is_client_error() || response.status.is_server_error() {
            Err(HttpClientError::Api(request.error_reader.read(response)?))
        } else {
            Err!("Server returned an error: {}", response.status)
        }
    }

    fn send_request(&self, method: Method, url: &str, headers: Headers, body: Option<Body>,
                    timeout: Duration) -> GenericResult<HttpResponse>
    {
        let client = Client::builder().timeout(timeout).build().map_err(|e| format!(
            "Unable to create HTTP client: {}", e))?;

        let mut request = client.request(method, url).headers(headers);
        if let Some(body) = body {
            request = request.body(body);
        }

        let mut response = request.send()?;
        let status = response.status();

        let mut body = Vec::new();
        response.copy_to(&mut body)?;

        if status == StatusCode::NO_CONTENT {
            trace!("Got {} response.", status);
        } else {
            trace!("Got {} response: {}", status,
               String::from_utf8_lossy(&body).trim_end_matches('\n'));
        }

        Ok(HttpResponse {
            status, body,
            headers: response.headers().clone(),
        })
    }
}

#[derive(Debug)]
pub enum HttpClientError<T> {
    Generic(String),
    Api(T),
}

impl<T: Error> Error for HttpClientError<T> {
}

impl<T: fmt::Display> fmt::Display for HttpClientError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            HttpClientError::Generic(ref err) => write!(f, "{}", err),
            HttpClientError::Api(ref err) => err.fmt(f),
        }
    }
}

impl<T> From<HttpRequestBuildingError> for HttpClientError<T> {
    fn from(err: HttpRequestBuildingError) -> HttpClientError<T> {
        HttpClientError::Generic(err.to_string())
    }
}

impl<T> From<String> for HttpClientError<T> {
    fn from(err: String) -> HttpClientError<T> {
        HttpClientError::Generic(err)
    }
}

impl<T> From<Box<dyn Error + Send + Sync>> for HttpClientError<T> {
    fn from(err: Box<dyn Error + Send + Sync>) -> HttpClientError<T> {
        HttpClientError::Generic(err.to_string())
    }
}