use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use hyper::Body;
use hyper::header::{Authorization, Bearer, Headers};
use serde::{ser, de};
use serde_json;

use core::{EmptyResult, GenericResult};
use hash::{Hasher, ChunkedSha256};
use http_client::{HttpClient, EmptyResponse, HttpClientError};
use provider::{Provider, ProviderType, ReadProvider, WriteProvider, File, FileType};
use stream_splitter::{ChunkStreamReceiver, ChunkStream};

const OAUTH_ENDPOINT: &'static str = "https://accounts.google.com/o/oauth2";
const API_ENDPOINT: &'static str = "https://www.googleapis.com/drive/v3";
// FIXME
const CONTENT_ENDPOINT: &'static str = "https://content.dropboxapi.com/2";

pub struct GoogleDrive {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    access_token: RefCell<Option<AccessToken>>,

    client: HttpClient,
}

struct AccessToken {
    token: String,
    expire_time: Instant,
}

impl GoogleDrive {
    pub fn new(client_id: &str, client_secret: &str, refresh_token: &str) -> GenericResult<GoogleDrive> {
        Ok(GoogleDrive {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
            refresh_token: refresh_token.to_owned(),
            access_token: RefCell::new(None),

            // FIXME
            client: HttpClient::new()?
                .with_default_header(Authorization(Bearer {token: refresh_token.to_owned()}))
        })
    }

    fn access_token(&self) -> Result<String, HttpClientError<ApiError>> {
        let mut access_token = self.access_token.borrow_mut();

        if let Some(ref access_token) = *access_token {
            let now = Instant::now();

            if access_token.expire_time < now &&
                now.duration_since(access_token.expire_time) > Duration::from_secs(1) // FIXME: Request timeout here?
            {
                return Ok(access_token.token.to_owned());
            }
        }

        debug!("Obtaining a new Google Drive access token...");

        #[derive(Serialize)]
        struct Request<'a> {
            client_id: &'a str,
            client_secret: &'a str,
            refresh_token: &'a str,
            grant_type: &'a str,
        }

        #[derive(Deserialize)]
        struct Response {
            access_token: String,
            expires_in: u64,
        }

        let request_time = Instant::now();

        let response: Response = self.oauth_request("/token", &Request {
            client_id: &self.client_id,
            client_secret: &self.client_secret,
            refresh_token: &self.refresh_token,
            grant_type: "refresh_token",
        }).map_err(HttpClientError::generic_from)?;

        *access_token = Some(AccessToken {
            token: response.access_token.to_owned(),
            expire_time: request_time + Duration::from_secs(response.expires_in)
        });

        Ok(response.access_token)
    }

    fn oauth_request<I, O>(&self, path: &str, request: &I) -> Result<O, HttpClientError<OauthApiError>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
    {
        let url = OAUTH_ENDPOINT.to_owned() + path;
        return self.client.form_request(&url, request, Duration::from_secs(5));
    }

    // FIXME
    fn api_request<I, O>(&self, path: &str, request: &I) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
    {
        self.access_token().unwrap();
        let url = API_ENDPOINT.to_owned() + path;
        return self.client.json_request(&url, request, Duration::from_secs(15));
    }

    // FIXME
    fn content_request<I, B, O>(&self, path: &str, request: &I, body: B) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              B: Into<Body>,
              O: de::DeserializeOwned,
    {
        let url = CONTENT_ENDPOINT.to_owned() + path;
        let mut headers = Headers::new();

        let request_json = serde_json::to_string(request).map_err(HttpClientError::generic_from)?;
        headers.set_raw("Dropbox-API-Arg", request_json);

        return self.client.upload_request(&url, &headers, body, Duration::from_secs(60 * 60));
    }
}

impl Provider for GoogleDrive {
    fn name(&self) -> &'static str {
        "Google Drive"
    }

    fn type_(&self) -> ProviderType {
        ProviderType::Cloud
    }
}

impl ReadProvider for GoogleDrive {
    // FIXME
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>> {
        #[derive(Serialize)]
        struct Request<'a> {
            path: &'a str,
        }

        #[derive(Serialize)]
        struct ContinueRequest<'a> {
            cursor: &'a str,
        }

        #[derive(Deserialize)]
        struct Response {
            entries: Vec<Entry>,
            cursor: String,
            has_more: bool,
        }

        #[derive(Deserialize)]
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
                self.api_request("/files/list_folder/continue", &ContinueRequest {
                    cursor: &cursor
                })
            } else {
                let response = self.api_request("/files/list_folder", &Request {
                    path: path
                });

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
                        "file" => FileType::File,
                        _ => FileType::Other,
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

impl WriteProvider for GoogleDrive {
    // FIXME
    fn hasher(&self) -> Box<Hasher> {
        Box::new(ChunkedSha256::new(4 * 1024 * 1024))
    }

    // FIXME
    fn max_request_size(&self) -> u64 {
        150 * 1024 * 1024
    }

    // FIXME
    fn create_directory(&self, path: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            path: &'a str,
        }

        let _: EmptyResponse = self.api_request("/files/create_folder_v2", &Request {
            path: path
        })?;

        Ok(())
    }

    // FIXME
    fn upload_file(&self, temp_path: &str, path: &str, chunk_streams: ChunkStreamReceiver) -> EmptyResult {
        #[derive(Serialize)]
        struct StartRequest {
        }

        #[derive(Deserialize)]
        struct StartResponse {
            session_id: String,
        }

        #[derive(Serialize)]
        struct AppendRequest<'a> {
            cursor: Cursor<'a>,
        }

        #[derive(Serialize)]
        struct FinishRequest<'a> {
            cursor: Cursor<'a>,
            commit: Commit<'a>,
        }

        #[derive(Deserialize)]
        struct FinishResponse {
            content_hash: String,
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

        let start_response: StartResponse = self.content_request(
            "/files/upload_session/start", &StartRequest{}, "")?;

        for result in chunk_streams.iter() {
            match result {
                Ok(ChunkStream::Stream(offset, chunk_stream)) => {
                    let _: Option<EmptyResponse> = self.content_request(
                        "/files/upload_session/append_v2", &AppendRequest {
                            cursor: Cursor {
                                session_id: &start_response.session_id,
                                offset: offset,
                            },
                        }, chunk_stream)?;
                },
                Ok(ChunkStream::EofWithCheckSum(size, checksum)) => {
                    let finish_response: FinishResponse = self.content_request(
                        "/files/upload_session/finish", &FinishRequest {
                            cursor: Cursor {
                                session_id: &start_response.session_id,
                                offset: size,
                            },
                            commit: Commit {
                                path: temp_path,
                                mode: "overwrite",
                            },
                        }, "")?;

                    if finish_response.content_hash != checksum {
                        if let Err(err) = self.delete(temp_path) {
                            error!("Failed to delete a temporary {:?} file from {}: {}.",
                                temp_path, self.name(), err);
                        }
                        return Err("Checksum mismatch".into());
                    }

                    return Ok(())
                }
                Err(err) => return Err(err.into()),
            }
        }

        Err!("Chunk stream sender has been closed without a termination message")
    }

    // FIXME
    fn delete(&self, path: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            path: &'a str,
        }

        let _: EmptyResponse = self.api_request("/files/delete_v2", &Request {
            path: path
        })?;

        Ok(())
    }
}

// FIXME
#[derive(Debug, Deserialize)]
struct ApiError {
    error: RouteError,
    error_summary: String,
}

// FIXME
#[derive(Debug, Deserialize)]
struct RouteError {
    #[serde(rename = ".tag")]
    tag: Option<String>,
    path: Option<PathError>,
}

// FIXME
#[derive(Debug, Deserialize)]
struct PathError {
    #[serde(rename = ".tag")]
    tag: String,
}

impl Error for ApiError {
    // FIXME
    fn description(&self) -> &str {
        "Dropbox API error"
    }
}

impl fmt::Display for ApiError {
    // FIXME
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Dropbox API error: {}",
               self.error_summary.trim_right_matches(|c| c == '.' || c == '/'))
    }
}

#[derive(Debug, Deserialize)]
struct OauthApiError {
    error_description: String,
}

impl Error for OauthApiError {
    fn description(&self) -> &str {
        "Google OAuth error"
    }
}

impl fmt::Display for OauthApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Google OAuth error: {}", self.error_description)
    }
}