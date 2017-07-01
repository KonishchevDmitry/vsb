use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::io::{self, Write};

use core::{EmptyResult, GenericResult};

use futures::Stream;
use hyper::{Client, Method, Request, Headers, Response, StatusCode, Chunk};
use hyper::client::HttpConnector;
use hyper::header::{Header, UserAgent, Authorization, Bearer, ContentLength, ContentType};
use hyper_tls::HttpsConnector;
use mime;
use serde::{ser, de};
use serde_json;
use tokio_core::reactor::Core;

// FIXME: timeouts
pub struct HttpClient {
    core: RefCell<Core>,
    client: Client<HttpsConnector<HttpConnector>>,
    default_headers: Headers,
}

impl HttpClient {
    pub fn new() -> GenericResult<HttpClient> {
        let mut default_headers = Headers::new();
        default_headers.set(UserAgent::new("pyvsb-to-cloud"));

        let core = Core::new()?;
        let handle = core.handle();

        Ok(HttpClient {
            core: RefCell::new(core),
            client: Client::configure().connector(HttpsConnector::new(1, &handle)?).build(&handle),
            default_headers: default_headers,
        })
    }

    pub fn with_default_header<H: Header>(mut self, header: H) -> HttpClient {
        self.default_headers.set(header);
        self
    }

    pub fn json_request<Req, Resp, E>(&self, url: &str, request: &Req) -> Result<Resp, HttpClientError<E>>
        where Req: ser::Serialize,
              Resp: de::DeserializeOwned,
              E: de::DeserializeOwned + fmt::Debug,
    {
        let method = Method::Post;
        let url = url.parse().map_err(HttpClientError::generic_from)?;
        let request_json = serde_json::to_string(request).map_err(HttpClientError::generic_from)?;
        trace!("Sending {method} {url} {request}...", method=method, url=url, request=request_json);

        let mut core = self.core.borrow_mut();
        let mut http_request = Request::new(method, url);

        http_request.headers_mut().extend(self.default_headers.iter());
        http_request.headers_mut().set(ContentType::json());
        http_request.headers_mut().set(ContentLength(request_json.len() as u64));

        http_request.set_body(request_json);

        let response: Response = core.run(self.client.request(http_request))
            .map_err(HttpClientError::generic_from)?;

        // Response::body() borrows Response, so we have to store all fields that we need later
        let status = response.status();
        let content_type = response.headers().get::<ContentType>().map(
            |header_ref| header_ref.clone());

        // FIXME: Use sync methods (send())
        // FIXME: Limit size
        let body: Chunk = core.run(response.body().concat2())
            .map_err(HttpClientError::generic_from)?;

        let body = String::from_utf8(body.to_vec()).map_err(|e|
            format!("Got an invalid response from server: {}", e))?;
        trace!("Got {} response: {}", status, body);

        if status != StatusCode::Ok {
            return if status.is_client_error() || status.is_server_error() {
                Err(HttpClientError::Api(
                    parse_error(status, content_type, &body).map_err(HttpClientError::generic_from)?))
            } else {
                Err!("Server returned an error: {}", status)
            }
        }

        Ok(serde_json::from_str(&body).map_err(|e|
            format!("Got an invalid response from server: {}", e))?)
    }
}

#[derive(Debug)]
pub enum HttpClientError<T> {
    Generic(String),
    Api(T),
}

// FIXME
impl<T> HttpClientError<T> {
    fn generic_from<E: ToString>(error: E) -> HttpClientError<T> {
        HttpClientError::Generic(error.to_string())
    }
//    fn generic_from<E: Error>(error: E) -> HttpClientError<T> {
//        HttpClientError::Generic(error.to_string())
//    }
}

// FIXME
impl<T: fmt::Debug> Error for HttpClientError<T> {
    fn description(&self) -> &str {
        "HTTP client error"
    }
}

// FIXME
impl<T> fmt::Display for HttpClientError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::HttpClientError::*;
        match *self {
            Generic(ref err) => write!(f, "{}", err),
            Api(ref err) => write!(f, "HACK"),  // FIXME
        }
    }
}

// FIXME
impl<T> From<String> for HttpClientError<T> {
    fn from(err: String) -> HttpClientError<T> {
        return HttpClientError::Generic(err)
    }
}

fn parse_error<T>(status: StatusCode, content_type: Option<ContentType>, body: &str) -> GenericResult<T>
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