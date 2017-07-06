use std::error::Error;
use std::fmt;
use std::thread;

use futures::{Future, Sink};
use hyper::{self, Body, Chunk};
use hyper::header::{Authorization, Bearer, Headers};
use serde::ser;
use serde::de;
use serde_json;

use core::{EmptyResult, GenericResult};
use http_client::{HttpClient, EmptyResponse, HttpClientError};
use provider::{Provider, File, FileType};

const API_ENDPOINT: &'static str = "https://api.dropboxapi.com/2";
const CONTENT_ENDPOINT: &'static str = "https://content.dropboxapi.com/2";

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

    fn content_request<I, B, O>(&self, path: &str, request: &I, body: B) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              B: Into<Body>,
              O: de::DeserializeOwned,
    {
        let url = CONTENT_ENDPOINT.to_owned() + path;
        let mut headers = Headers::new();

        let request_json = serde_json::to_string(request).map_err(HttpClientError::generic_from)?;
        headers.set_raw("Dropbox-API-Arg", request_json);

        return self.client.upload_request(&url, &headers, body);
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

    fn upload_file(&self, path: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct StartRequest {
        }

        #[derive(Debug, Deserialize)]
        struct StartResponse {
            session_id: String,
        }

        #[derive(Serialize)]
        struct AppendRequest<'a> {
            cursor: Cursor<'a>,
            // FIXME
            close: bool,
        }

        #[derive(Serialize)]
        struct FinishRequest<'a> {
            cursor: Cursor<'a>,
            commit: Commit<'a>,
        }

        #[derive(Serialize)]
        struct Cursor<'a> {
            session_id: &'a str,
            offset: u64,
        }

        #[derive(Serialize)]
        struct Commit<'a> {
            path: &'a str,
            mode: &'a str,
        }

        // FIXME
        use futures::sync::mpsc;
        let (mut tx, rx) = mpsc::channel(2);

        thread::spawn(|| {
            let data: Result<Chunk, hyper::Error> = Ok(From::from("a"));
            tx = tx.send(data).wait().unwrap();
            let data: Result<Chunk, hyper::Error> = Ok(From::from("b"));
            tx = tx.send(data).wait().unwrap();
            drop(tx);
        });

        let start_response: StartResponse = self.content_request("/files/upload_session/start", &StartRequest{}, "")?;

        let _: Option<EmptyResponse> = self.content_request("/files/upload_session/append_v2", &AppendRequest{
            cursor: Cursor {
                session_id: &start_response.session_id,
                offset: 0,
            },
            close: true,
        }, rx)?;

        let _: EmptyResponse = self.content_request("/files/upload_session/finish", &FinishRequest{
            cursor: Cursor {
                session_id: &start_response.session_id,
                offset: 2,
            },
            commit: Commit {
                path: "/test",
                mode: "overwrite",
            },
        }, "")?;

        Ok(())
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
