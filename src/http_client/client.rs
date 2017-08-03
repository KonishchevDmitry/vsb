use std::error::Error;
use std::fmt;
use std::io;
use std::time::{Instant, Duration};

use futures::{Future, Stream};
use hyper::{self, Client, Body, StatusCode, Chunk};
use hyper::header::{Header, UserAgent, ContentLength, ContentType};
use hyper_tls::HttpsConnector;
use log::LogLevel;
use mime;
use serde::{ser, de};
use serde_json;
use serde_urlencoded;
use tokio_core::reactor::{Core, Timeout};

// FIXME
pub use hyper::{Method, Headers};

use core::GenericResult;
use super::{Request, Response, RawResponseReader};

pub struct HttpClient {
    default_headers: Headers,
}

impl HttpClient {
    pub fn new() -> GenericResult<HttpClient> {
        let mut default_headers = Headers::new();
        default_headers.set(UserAgent::new("pyvsb-to-cloud"));

        Ok(HttpClient {
            default_headers: default_headers,
        })
    }

    pub fn with_default_header<H: Header>(mut self, header: H) -> HttpClient {
        self.default_headers.set(header);
        self
    }

    // FIXME: deprecate
    pub fn form_request<I, O, E>(&self, url: &str, request: &I, timeout: Duration) -> Result<O, HttpClientError<E>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let request = Request::<O, E>::new_json(Method::Post, url.to_owned(), timeout).with_form(request)
            .map_err(HttpClientError::generic_from)?;

        self.send(request).map_err(HttpClientError::generic_from)
    }

    // FIXME: deprecate
    pub fn json_request<I, O, E>(&self, method: Method, url: &str, request: &I, timeout: Duration) -> Result<O, HttpClientError<E>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let request = Request::<O, E>::new_json(method, url.to_owned(), timeout).with_json(request)
            .map_err(HttpClientError::generic_from)?;

        self.send(request).map_err(HttpClientError::generic_from)
    }

    // FIXME: deprecate
    pub fn upload_request<I, O, E>(&self, url: &str, headers: &Headers, body: I, timeout: Duration) -> Result<O, HttpClientError<E>>
        where I: Into<Body>,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let mut request = Request::<O, E>::new_json(Method::Post, url.to_owned(), timeout)
            .with_body(ContentType::octet_stream(), None, body)
            .map_err(HttpClientError::generic_from)?;

        // FIXME: trace
        request.headers.extend(headers.iter());

        self.send(request).map_err(HttpClientError::generic_from)
    }

    // FIXME
    pub fn send<R, E>(&self, request: Request<R, E>) -> Result<R, HttpClientError<E>> {
        // FIXME
        if log_enabled!(LogLevel::Trace) {
            let mut extra_info = String::new();

            if request.trace_headers.len() != 0 {
                extra_info += &format!("\n{}", request.trace_headers.iter()
                    .map(|header| header/*.to_string()*/.trim_right_matches("\r\n").to_owned())
                           .collect::<Vec<_>>().join("\n"));
            }

            if let Some(body) = request.trace_body {
                extra_info = extra_info + " " + &body;
            }

            trace!("Sending {method} {url}{extra_info}...",
                   method=request.method, url=request.url, extra_info=extra_info);
        }

        let mut headers = self.default_headers.clone();
        headers.extend(request.headers.iter());

        let response = self.send_request(
            request.method, &request.url, headers, request.body, request.timeout)
            .map_err(HttpClientError::generic_from)?; // FIXME

        if response.status.is_success() {
            Ok(request.reply_reader.read(response).map_err(HttpClientError::generic_from)?)
        } else if response.status.is_client_error() || response.status.is_server_error() {
            Err(HttpClientError::Api(
                request.error_reader.read(response).map_err(HttpClientError::generic_from)?))
        } else {
            Err!("Server returned an error: {}", response.status)
        }
    }

    fn send_request<I>(&self, method: Method, url: &str, headers: Headers, body: I,
                       timeout: Duration) -> GenericResult<Response>
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
        trace!("Got {} response: {}", status, String::from_utf8_lossy(&body));

        Ok(Response {
            status: status,
            headers: response_headers,
            body: body,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct EmptyResponse {
}

#[derive(Debug)]
pub enum HttpClientError<T> {
    Generic(String),
    Api(T),
}

impl<T> HttpClientError<T> {
    // FIXME: Do we need these conversions?
    pub fn generic_from<E: ToString>(error: E) -> HttpClientError<T> {
        HttpClientError::Generic(error.to_string())
    }
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

// FIXME: Do we need these conversions?
impl<T> From<String> for HttpClientError<T> {
    fn from(err: String) -> HttpClientError<T> {
        HttpClientError::Generic(err)
    }
}