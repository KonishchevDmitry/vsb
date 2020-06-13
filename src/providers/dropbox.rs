use std::error::Error;
use std::fmt;
use std::ops::Add;
use std::time::Duration;

use serde::{ser, de};
use serde_json;

use core::{EmptyResult, GenericResult};
use hash::{Hasher, ChunkedSha256};
use http_client::{HttpClient, HttpRequest, HttpRequestBuildingError, Method, Body, EmptyResponse,
                  HttpClientError, headers};
use provider::{Provider, ProviderType, ReadProvider, WriteProvider, File, FileType};
use stream_splitter::{ChunkStreamReceiver, ChunkStream};

const API_ENDPOINT: &str = "https://api.dropboxapi.com/2";
const API_REQUEST_TIMEOUT: u64 = 15;

const CONTENT_ENDPOINT: &str = "https://content.dropboxapi.com/2";
const CONTENT_REQUEST_TIMEOUT: u64 = 60 * 60;

pub struct Dropbox {
    client: HttpClient,
}

impl Dropbox {
    pub fn new(access_token: &str) -> GenericResult<Dropbox> {
        Ok(Dropbox {
            client: HttpClient::new()
                .with_default_header(headers::AUTHORIZATION, format!("Bearer {}", access_token))
                .map_err(|_| "Invalid access token")?
        })
    }

    fn rename_file(&self, src: &str, dst: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            from_path: &'a str,
            to_path: &'a str,
        }

        let _: EmptyResponse = self.api_request("/files/move_v2", &Request {
            from_path: src,
            to_path: dst,
        })?;

        Ok(())
    }

    fn api_request<I, O>(&self, path: &str, request: &I) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
    {
        self.client.send(HttpRequest::new_json(
            Method::POST, API_ENDPOINT.to_owned() + path,
            Duration::from_secs(API_REQUEST_TIMEOUT)
        ).with_json(request)?)
    }

    fn content_request<I, B, O>(&self, path: &str, request: &I, body: B) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              B: Into<Body>,
              O: de::DeserializeOwned,
    {
        let request_json = serde_json::to_string(request).map_err(HttpRequestBuildingError::new)?;

        let http_request = HttpRequest::new_json(
            Method::POST, CONTENT_ENDPOINT.to_owned() + path,
            Duration::from_secs(CONTENT_REQUEST_TIMEOUT))
            .with_header("Dropbox-API-Arg", request_json)?
            .with_body("application/octet-stream", body)?;

        self.client.send(http_request)
    }
}

impl Provider for Dropbox {
    fn name(&self) -> &'static str {
        "Dropbox"
    }

    fn type_(&self) -> ProviderType {
        ProviderType::Cloud
    }
}

impl ReadProvider for Dropbox {
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
            let mut response: Response = if let Some(ref cursor) = cursor {
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

            for entry in response.entries.drain(..) {
                files.push(File {
                    name: entry.name,
                    type_: match entry.tag.as_str() {
                        "folder" => FileType::Directory,
                        "file" => FileType::File,
                        _ => FileType::Other,
                    },
                });
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

impl WriteProvider for Dropbox {
    fn hasher(&self) -> Box<dyn Hasher> {
        Box::new(ChunkedSha256::new(4 * 1024 * 1024))
    }

    fn max_request_size(&self) -> Option<u64> {
        Some(150 * 1024 * 1024)
    }

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

    fn upload_file(&self, directory_path: &str, temp_name: &str, name: &str,
                   chunk_streams: ChunkStreamReceiver) -> EmptyResult {
        let temp_path = directory_path.trim_end_matches('/').to_owned().add("/").add(temp_name);
        let path = directory_path.trim_end_matches('/').to_owned().add("/").add(name);

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
        }

        #[derive(Serialize)]
        struct FinishRequest<'a> {
            cursor: Cursor<'a>,
            commit: Commit<'a>,
        }

        #[derive(Debug, Deserialize)]
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
                                path: &temp_path,
                                mode: "overwrite",
                            },
                        }, "")?;

                    if finish_response.content_hash != checksum {
                        if let Err(err) = self.delete(&temp_path) {
                            error!("Failed to delete a temporary {:?} file from {}: {}.",
                                   temp_path, self.name(), err);
                        }
                        return Err("Checksum mismatch".into());
                    }

                    return self.rename_file(&temp_path, &path);
                }
                Err(err) => return Err(err.into()),
            }
        }

        Err!("Chunk stream sender has been closed without a termination message")
    }

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
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Dropbox API error: {}", self.error_summary.trim_end_matches(|c| c == '.' || c == '/'))
    }
}