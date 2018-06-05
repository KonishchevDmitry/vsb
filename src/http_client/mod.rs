mod readers;
mod request;
mod response;

use std::error::Error;
use std::fmt;
use std::io;
use std::time::{Instant, Duration};

use futures::{Future, Stream};
use hyper::{self, Client, Body, Chunk};
use hyper::header::{Header, UserAgent};
use hyper_tls::HttpsConnector;
use log;
use tokio_core::reactor::{Core, Timeout};

use core::GenericResult;

pub use hyper::{Method, Headers, StatusCode};
pub use self::request::*;
pub use self::response::*;
pub use self::readers::*;

pub struct HttpClient {
    default_headers: Headers,
}

impl HttpClient {
    pub fn new() -> HttpClient {
        let mut default_headers = Headers::new();
        default_headers.set(UserAgent::new("pyvsb-to-cloud"));

        HttpClient {
            default_headers: default_headers,
        }
    }

    pub fn with_default_header<H: Header>(mut self, header: H) -> HttpClient {
        self.default_headers.set(header);
        self
    }

    pub fn send<R, E>(&self, request: HttpRequest<R, E>) -> Result<R, HttpClientError<E>> {
        let mut headers = self.default_headers.clone();
        headers.extend(request.headers.iter());

        if log_enabled!(log::Level::Trace) {
            let mut extra_info = String::new();

            if headers.len() != 0 {
                extra_info += "\n";
                extra_info += &headers.iter()
                    .map(|header| header.to_string().trim_right_matches("\r\n").to_owned())
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

    fn send_request<I>(&self, method: Method, url: &str, headers: Headers, body: I,
                       timeout: Duration) -> GenericResult<HttpResponse>
        where I: Into<Body>
    {
        let url = url.parse()?;

        let mut http_request = hyper::Request::new(method, url);
        *http_request.headers_mut() = headers;
        http_request.set_body(body);

        // Attention:
        // We create a new event loop per each request because hyper has a bug due to which in case
        // when server responds before we complete sending our request (for example with 413 Payload
        // Too Large status code), request body gets leaked probably somewhere in connection pool,
        // so body's mpsc::Receiver doesn't gets closed and sender hangs on it forever.
        let mut core = Core::new()?;
        let handle = core.handle();
        let https_connector = HttpsConnector::new(1, &handle)?;
        let client = Client::configure().connector(https_connector).build(&handle);

        // Sadly, but for now it seems to be impossible to set socket or per-chunk timeout in hyper,
        // so we have to always operate with request timeout.
        let timeout_time = Instant::now() + timeout;
        let timeout = Timeout::new_at(timeout_time, &handle)?.and_then(|_| {
            Err(io::Error::new(io::ErrorKind::TimedOut, "HTTP request timeout"))
        }).map_err(Into::into);

        let response: hyper::Response = match core.run(client.request(http_request).select(timeout)) {
            Ok((response, _)) => Ok(response),
            Err((err, _)) => Err(err),
        }?;

        // Response::body() borrows Response, so we have to store all fields that we need later
        let status = response.status();
        let response_headers = response.headers().clone();

        let timeout = Timeout::new_at(timeout_time, &handle)?.and_then(|_| {
            Err(io::Error::new(io::ErrorKind::TimedOut, "HTTP response receiving timeout"))
        }).map_err(Into::into);

        let body: Chunk = match core.run(response.body().concat2().select(timeout)) {
            Ok((body, _)) => Ok(body),
            Err((err, _)) => Err(err),
        }?;
        let body = body.to_vec();
        trace!("Got {} response: {}", status,
               String::from_utf8_lossy(&body).trim_right_matches('\n'));

        Ok(HttpResponse {
            status: status,
            headers: response_headers,
            body: body,
        })
    }
}

#[derive(Debug)]
pub enum HttpClientError<T> {
    Generic(String),
    Api(T),
}

impl<T: Error> Error for HttpClientError<T> {
    fn description(&self) -> &str {
        match *self {
            HttpClientError::Generic(_) => "HTTP client error",
            HttpClientError::Api(ref e) => e.description(),
        }
    }
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

impl<T> From<Box<Error + Send + Sync>> for HttpClientError<T> {
    fn from(err: Box<Error + Send + Sync>) -> HttpClientError<T> {
        HttpClientError::Generic(err.to_string())
    }
}