use std::error::Error;
use std::fmt;

use hyper::header::{Authorization, Bearer};
use serde::ser;
use serde::de;

use core::GenericResult;
use http_client::{HttpClient, HttpClientError};
use provider::{Provider, File, FileType};

const API_ENDPOINT: &'static str = "https://api.dropboxapi.com/2";

pub struct Dropbox {
    client: HttpClient,
}

impl Dropbox {
    pub fn new(access_token: &str) -> GenericResult<Dropbox> {
        Ok(Dropbox {
            client: HttpClient::new()?
                .with_default_header(Authorization(Bearer {token: access_token.to_owned()}))
        })
    }

    fn api_request<I, O>(&self, path: &str, request: &I) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
    {
        let url = API_ENDPOINT.to_owned() + path;
        return self.client.json_request(&url, request);
    }
}

impl Provider for Dropbox {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>> {
        #[derive(Serialize)]
        struct Request<'a> {
            path: &'a str,
        }

        #[derive(Serialize)]
        struct ContinueRequest<'a> {
            cursor: &'a str,
        }

        #[derive(Debug, Deserialize)]
        struct Response {
            entries: Vec<Entry>,
            cursor: String,
            has_more: bool,
        }

        #[derive(Debug, Deserialize)]
        struct Entry {
            #[serde(rename = ".tag")]
            tag: String,
            name: String,
        }

        let mut cursor: Option<String> = None;
        let (mut page, page_limit) = (1, 1000);
        let mut files = Vec::new();

        loop {
            let response: Response = if let Some(ref cursor) = cursor {
                self.api_request("/files/list_folder/continue", &ContinueRequest{cursor: &cursor})
            } else {
                let response = self.api_request("/files/list_folder", &Request{path: path});

                if let Err(HttpClientError::Api(ref e)) = response {
                    if e.error.tag.as_ref().map(|tag| tag == "path").unwrap_or_default() {
                        if let Some(ref e) = e.error.path {
                            if e.tag == "not_found" {
                                return Ok(None);
                            }
                        }
                    }
                }

                response
            }?;

            for ref entry in &response.entries {
                files.push(File {
                    name: entry.name.clone(),
                    type_: match entry.tag.as_str() {
                        "folder" => FileType::Directory,
                        _ => FileType::File,
                    },
                })
            }

            if !response.has_more {
                break;
            }

            if page >= page_limit {
                return Err!("Directory listing page limit has exceeded");
            }

            cursor = Some(response.cursor);
            page += 1;
        }

        Ok(Some(files))
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
    tag: Option<String>,
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
