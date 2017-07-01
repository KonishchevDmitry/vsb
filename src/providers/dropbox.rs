use std::env;
use std::error::Error;
use std::fmt;

use hyper::header::{Authorization, Bearer};
use serde::ser;
use serde::de;

use core::{EmptyResult, GenericResult};
use http_client::{HttpClient, HttpClientError};

const API_ENDPOINT: &'static str = "https://api.dropboxapi.com/2";

pub struct Dropbox {
    client: HttpClient,
}

impl Dropbox {
    pub fn new() -> GenericResult<Dropbox> {
        // FIXME
        let access_token = env::var("DROPBOX_ACCESS_TOKEN").unwrap();

        Ok(Dropbox {
            client: HttpClient::new().unwrap() // FIXME
                .with_default_header(Authorization(Bearer {token: access_token.to_owned()}))
        })
    }

    pub fn test(&self) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            path: &'a str,
        }

        #[derive(Debug, Deserialize)]
        struct Response {
        }

        let result = self.api_request("/files/list_folder", &Request{path: "/invalid"});

        if let Err(HttpClientError::Api(ref e)) = result {
            if e.error.tag == "path" {
                if let Some(ref e) = e.error.path {
                    error!(">>> {}", e.tag);
                }
            }
        }

        let result = result?;

        info!("Response: {:?}", result);

        Ok(())
    }

    fn api_request<I, O>(&self, path: &str, request: &I) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
    {
        let url = API_ENDPOINT.to_owned() + path;
        return self.client.json_request(&url, request);
    }
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: RouteError,
    error_summary: String,
}

#[derive(Debug, Deserialize)]
struct RouteError {
    #[serde(rename = ".tag")]
    tag: String,
    path: Option<PathError>,
}

#[derive(Debug, Deserialize)]
struct PathError {
    #[serde(rename = ".tag")]
    tag: String,
}

impl Error for ApiError {
    fn description(&self) -> &str {
        "Dropbox API error"
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Dropbox API error: {}",
               self.error_summary.trim_right_matches(|c| c == '.' || c == '/'))
    }
}
