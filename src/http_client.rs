use std::cell::RefCell;
use std::error::Error;
use std::fmt;

use core::GenericResult;

use futures::Stream;
use hyper::{Client, Method, Request, Headers, Response, StatusCode, Chunk};
use hyper::client::HttpConnector;
use hyper::header::{Header, UserAgent, ContentLength, ContentType};
use hyper::Body;
use hyper_tls::HttpsConnector;
use mime;
use serde::{ser, de};
use serde_json;
use tokio_core::reactor::Core;

// FIXME: timeouts
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

    pub fn json_request<I, O, E>(&self, url: &str, request: &I) -> Result<O, HttpClientError<E>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let method = Method::Post;
        let request_json = serde_json::to_string(request).map_err(HttpClientError::generic_from)?;
        trace!("Sending {method} {url} {request}...", method=method, url=url, request=request_json);

        let mut headers = self.default_headers.clone();
        headers.set(ContentType::json());
        headers.set(ContentLength(request_json.len() as u64));

        self.process_request(method, url, headers, request_json)
    }

    pub fn upload_request<I, O, E>(&self, url: &str, headers: &Headers, body: I) -> Result<O, HttpClientError<E>>
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

        self.process_request(method, url, request_headers, body)
    }

    fn process_request<I, O, E>(&self, method: Method, url: &str, headers: Headers, body: I) -> Result<O, HttpClientError<E>>
        where I: Into<Body>,
              O: de::DeserializeOwned,
              E: de::DeserializeOwned + Error,
    {
        let url = url.parse().map_err(HttpClientError::generic_from)?;

        let mut http_request = Request::new(method, url);
        *http_request.headers_mut() = headers;
        http_request.set_body(body);

        // Attention:
        // We create a new event loop per each request because hyper has a bug due to which in case
        // when server responds before we complete sending our request (for example with 413 Payload
        // Too Large status code), request body gets leaked probably somewhere in connection pool,
        // so body's mpsc::Receiver doesn't gets closed and sender hangs on it forever.
        let mut core = Core::new().map_err(HttpClientError::generic_from)?;
        let handle = core.handle();
        let https_connector = HttpsConnector::new(1, &handle)
            .map_err(HttpClientError::generic_from)?;
        let client =  Client::configure().connector(https_connector).build(&handle);

        let response: Response = core.run(client.request(http_request))
            .map_err(HttpClientError::generic_from)?;

        // Response::body() borrows Response, so we have to store all fields that we need later
        let status = response.status();
        let content_type = response.headers().get::<ContentType>().map(
            |header_ref| header_ref.clone());

        // FIXME: Limit size
        let body: Chunk = core.run(response.body().concat2())
            .map_err(HttpClientError::generic_from)?;

        let body = String::from_utf8(body.to_vec()).map_err(|e|
            format!("Got an invalid response from server: {}", e))?;
        trace!("Got {} response: {}", status, body);

        if status != StatusCode::Ok {
            return if status.is_client_error() || status.is_server_error() {
                Err(HttpClientError::Api(
                    parse_api_error(status, content_type, &body).map_err(HttpClientError::generic_from)?))
            } else {
                Err!("Server returned an error: {}", status)
            }
        }

        Ok(serde_json::from_str(&body).map_err(|e|
            format!("Got an invalid response from server: {}", e))?)
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

// FIXME
impl<T> HttpClientError<T> {
    pub fn generic_from<E: ToString>(error: E) -> HttpClientError<T> {
        HttpClientError::Generic(error.to_string())
    }
}

impl<T: Error> Error for HttpClientError<T> {
    fn description(&self) -> &str {
        match *self {
            HttpClientError::Generic(_) => "HTTP client generic error",
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

// FIXME
impl<T> From<String> for HttpClientError<T> {
    fn from(err: String) -> HttpClientError<T> {
        return HttpClientError::Generic(err)
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