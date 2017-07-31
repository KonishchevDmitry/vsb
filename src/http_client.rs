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

    pub fn form_request<I, O, E>(&self, url: &str, request: &I, timeout: Duration) -> Result<O, HttpClientError<E>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let method = Method::Post;
        let body = serde_urlencoded::to_string(request).map_err(HttpClientError::generic_from)?;
        trace!("Sending {method} {url} {body}...", method=method, url=url, body=body);

        let mut headers = self.default_headers.clone();
        headers.set(ContentType::form_url_encoded());
        headers.set(ContentLength(body.len() as u64));

        Ok(self.process_request(method, url, headers, body, timeout)?.1)
    }

    // FIXME: deprecate
    pub fn json_request<I, O, E>(&self, method: Method, url: &str, request: &I, timeout: Duration) -> Result<O, HttpClientError<E>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let request = Request::new(method, url.to_owned(), timeout).with_json(request)
            .map_err(HttpClientError::generic_from)?;
        Ok(self.request(request)?.1)
    }

    pub fn upload_request<I, O, E>(&self, url: &str, headers: &Headers, body: I, timeout: Duration) -> Result<O, HttpClientError<E>>
        where I: Into<Body>,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let method = Method::Post;

        if headers.len() == 0 {
            trace!("Sending {method} {url}...", method=method, url=url);
        } else {
            trace!("Sending {method} {url}:\n{headers}", method=method, url=url,
                   headers=headers.iter().map(|header| header.to_string().trim_right_matches("\r\n").to_owned())
                       .collect::<Vec<_>>().join("\n"));
        }

        let mut request_headers = self.default_headers.clone();
        request_headers.set(ContentType::octet_stream());
        request_headers.extend(headers.iter());

        Ok(self.process_request(method, url, request_headers, body, timeout)?.1)
    }

    // FIXME: Deprecate all specialized methods
    // FIXME: Return response object?
    pub fn request<O, E>(&self, request: Request) -> Result<(Headers, O), HttpClientError<E>>
        where O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        if log_enabled!(LogLevel::Trace) {
            let mut extra_info = String::new();

            if let Some(body) = request.trace_body {
                extra_info = extra_info + " " + &body;
            }

            trace!("Sending {method} {url}{extra_info}...",
                   method=request.method, url=request.url, extra_info=extra_info);
        }

        let mut headers = self.default_headers.clone();
        headers.extend(request.headers.iter());

        self.process_request(request.method, &request.url, headers, request.body, request.timeout)
    }

    // FIXME
    fn process_request<I, O, E>(&self, method: Method, url: &str, headers: Headers, body: I,
                                timeout: Duration
    ) -> Result<(Headers, O), HttpClientError<E>>
        where I: Into<Body>,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let (status, response_headers, body) = self.send_request(method, url, headers, body, timeout)
            // FIXME
            .map_err(HttpClientError::generic_from)?;

        let content_type = response_headers.get::<ContentType>().map(
            |header_ref| header_ref.clone());

        if status != StatusCode::Ok {
            return if status.is_client_error() || status.is_server_error() {
                Err(HttpClientError::Api(parse_api_error(status, content_type, &body)
                    .map_err(HttpClientError::generic_from)?))
            } else {
                Err!("Server returned an error: {}", status)
            }
        }

        let result = serde_json::from_str(&body).map_err(|e|
            format!("Got an invalid response from server: {}", e))?;

        Ok((response_headers, result))
    }

    // FIXME
    pub fn raw_request(&self, request: Request) -> Result<(Headers, String), HttpClientError<String>> // FIXME: Error type
    {
        let mut headers = self.default_headers.clone();
        headers.extend(request.headers.iter());

        // FIXME: logging
        let (status, response_headers, body) = self.send_request(
            request.method, &request.url, headers, request.body, request.timeout)?;

        if status != StatusCode::Ok {
            return Err!("Server returned an error: {}", status);
        }

        Ok((response_headers, body))
    }

    // FIXME
    fn send_request<I>(&self, method: Method, url: &str, headers: Headers, body: I,
                       timeout: Duration
    ) -> Result<(StatusCode, Headers, String), HttpClientError<String>> // FIXME: Error type
        where I: Into<Body>
    {
        let url = url.parse().map_err(HttpClientError::generic_from)?;

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
        let https_connector = HttpsConnector::new(1, &handle).map_err(HttpClientError::generic_from)?;
        let client =  Client::configure().connector(https_connector).build(&handle);

        // Sadly, but for now it seems to be impossible to set socket or per-chunk timeout in hyper,
        // so we have to always operate with request timeout.
        let timeout_time = Instant::now() + timeout;
        let timeout = Timeout::new_at(timeout_time, &handle)?.and_then(|_| {
            Err(io::Error::new(io::ErrorKind::TimedOut, "HTTP request timeout"))
        }).map_err(|e| {
            e.into()
        });

        let response: Response = match core.run(client.request(http_request).select(timeout)) {
            Ok((response, _)) => Ok(response),
            Err((err, _)) => Err(err),
        }?;

        // Response::body() borrows Response, so we have to store all fields that we need later
        let status = response.status();
        let response_headers = response.headers().clone();

        let timeout = Timeout::new_at(timeout_time, &handle)?.and_then(|_| {
            Err(io::Error::new(io::ErrorKind::TimedOut, "HTTP response receiving timeout"))
        }).map_err(|e| {
            e.into()
        });

        let body: Chunk = match core.run(response.body().concat2().select(timeout)) {
            Ok((body, _)) => Ok(body),
            Err((err, _)) => Err(err),
        }?;

        let body = String::from_utf8(body.to_vec()).map_err(|e|
            format!("Got an invalid response from server: {}", e))?;
        trace!("Got {} response: {}", status, body);

        Ok((status, response_headers, body))
    }
}

pub struct Request {
    method: Method,
    url: String,
    headers: Headers,
    body: Option<Body>,
    timeout: Duration,

    trace_body: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct EmptyResponse {
}

#[derive(Debug)]
pub enum HttpClientError<T> {
    Generic(String),
    Api(T),
}

impl<T> HttpClientError<T> {
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

impl<T> From<String> for HttpClientError<T> {
    fn from(err: String) -> HttpClientError<T> {
        HttpClientError::Generic(err)
    }
}

impl<T> From<io::Error> for HttpClientError<T> {
    fn from(err: io::Error) -> HttpClientError<T> {
        HttpClientError::generic_from(err)
    }
}

impl<T> From<hyper::Error> for HttpClientError<T> {
    fn from(err: hyper::Error) -> HttpClientError<T> {
        HttpClientError::generic_from(err)
    }
}

fn parse_api_error<T>(status: StatusCode, content_type: Option<ContentType>, body: &str) -> GenericResult<T>
    where T: de::DeserializeOwned
{
    let content_type = content_type.ok_or_else(|| format!(
        "Server returned {} error with an invalid content type", status))?;

    if content_type.type_() == mime::TEXT && content_type.subtype() == mime::PLAIN {
        let mut error = body.lines().next().unwrap_or("").trim_right_matches('.').trim().to_owned();
        if error.is_empty() {
            error = status.to_string();
        }
        return Err!("Server returned an error: {}", error);
    } else if content_type.type_() == mime::APPLICATION && content_type.subtype() == mime::JSON {
        return Ok(serde_json::from_str(body).map_err(|e| format!(
            "Server returned an invalid JSON response: {}", e))?);
    }

    Err!("Server returned {} error with an invalid content type: {}",
        status, content_type)
}