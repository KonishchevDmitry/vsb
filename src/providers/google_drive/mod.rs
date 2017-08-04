use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use hyper::Body;
use hyper::header::{Authorization, Bearer, Location, Headers, ContentType};
use mime::Mime;
use serde::{ser, de};
use serde_json;

use core::{EmptyResult, GenericResult};
use hash::{Hasher, ChunkedSha256};
use http_client::{HttpClient, Method, HttpRequest, HttpResponse, EmptyResponse, RawResponseReader,
                  JsonErrorReader, HttpStatusReader, HttpClientError};
use provider::{Provider, ProviderType, ReadProvider, WriteProvider, File, FileType};
use stream_splitter::{ChunkStreamReceiver, ChunkStream};

mod oauth;
use self::oauth::GoogleOauth;

const API_ENDPOINT: &'static str = "https://www.googleapis.com/drive/v3";
const API_REQUEST_TIMEOUT: u64 = 5;

const UPLOAD_ENDPOINT: &'static str = "https://www.googleapis.com/upload/drive/v3";

pub struct GoogleDrive {
    client: HttpClient,
    oauth: GoogleOauth,
}

type ApiResult<T> = Result<T, HttpClientError<GoogleDriveApiError>>;

impl GoogleDrive {
    pub fn new(client_id: &str, client_secret: &str, refresh_token: &str) -> GoogleDrive {
        GoogleDrive {
            client: HttpClient::new(),
            oauth: GoogleOauth::new(client_id, client_secret, refresh_token),
        }
    }

    // FIXME
    fn new_file(&self, path: &str) -> GenericResult<(String, String)> {
        if path == "/" {
            return Err!("File already exists")
        }

        if !path.starts_with('/') || path.ends_with('/') {
            return Err!("Invalid path: {:?}", path)
        }

        let mut components = path.rsplitn(2, '/');
        let name = components.next().unwrap().to_owned();
        let mut parent_path = components.next().unwrap();
        if parent_path.is_empty() {
            parent_path = "/";
        }

        let parent = match self.stat_path(&parent_path)? {
            Some(parent) => parent,
            None => return Err!("{:?} directory doesn't exist", parent_path),
        };

        if self.list_children(&parent.id)?.contains_key(&name) {
            return Err!("File already exists")
        }

        return Ok((parent.id, name))
    }

    fn stat_path(&self, path: &str) -> GenericResult<Option<GoogleDriveFile>> {
        let mut cur_path = "/".to_owned();
        let mut cur_dir_id = "root".to_owned();

        if path == "/" {
            let request_path = "/files/".to_owned() + &cur_dir_id;
            let file_metadata = self.client.send(self.api_request(Method::Get, &request_path)?)?;
            return Ok(Some(file_metadata));
        } else if !path.starts_with('/') || path.ends_with('/') {
            return Err!("Invalid path: {:?}", path);
        }

        let mut components = path.split('/');
        assert!(components.next().unwrap().is_empty());

        let mut component = components.next().unwrap();

        loop {
            let mut files = self.list_children(&cur_dir_id).map_err(|e| format!(
                "Error while reading {:?} directory: {}", cur_path, e))?;

            if !cur_path.ends_with('/') {
                cur_path.push('/');
            }
            cur_path += component;

            let file = match files.remove(component) {
                Some(mut files) => {
                    match files.len() {
                        0 => unreachable!(),
                        1 => files.pop().unwrap(),
                        _ => return Err!("{:?} path is unambiguous: it resolves to {} files",
                                    cur_path, files.len()),
                    }
                },
                None => return Ok(None),
            };

            component = match components.next() {
                Some(component) if component.is_empty() => return Err!("Invalid path: {:?}", path),
                Some(component) => component,
                None => return Ok(Some(file)),
            };

            if file.type_() != FileType::Directory {
                return Err!("{:?} is not a directory", cur_path);
            }

            cur_dir_id = file.id;
        }
    }

    fn list_children(&self, id: &str) -> GenericResult<HashMap<String, Vec<GoogleDriveFile>>> {
        #[derive(Serialize)]
        struct RequestParams {
            q: String,
            #[serde(rename = "pageToken")]
            page_token: Option<String>,
        }

        #[derive(Deserialize)]
        struct Response {
            files: Vec<GoogleDriveFile>,
            #[serde(rename = "incompleteSearch")]
            incomplete_search: bool,
            #[serde(rename = "nextPageToken")]
            next_page_token: Option<String>,
        }

        let mut request_params = RequestParams {
            q: format!("'{}' in parents and trashed = false", id),
            page_token: None,
        };
        let (mut page, page_limit) = (1, 1000);
        let mut files = HashMap::new();

        loop {
            let request = self.api_request(Method::Get, "/files")?.with_params(&request_params)?;
            let mut response: Response = self.client.send(request)?;

            if response.incomplete_search {
                return Err!("Got an incomplete result on directory listing")
            }

            for file in response.files.drain(..) {
                files.entry(file.name.clone()).or_insert_with(Vec::new).push(file);
            }

            if let Some(next_page_token) = response.next_page_token {
                if page >= page_limit {
                    return Err!("Directory listing page limit has exceeded");
                }

                request_params.page_token = Some(next_page_token);
                page += 1;
            } else {
                break;
            }
        }

        Ok(files)
    }

    fn access_token(&self) -> Result<String, GoogleDriveError> {
        self.oauth.get_access_token().map_err(|e| GoogleDriveError::Oauth(format!(
            "Unable obtain a Google OAuth token: {}", e)))
    }

    fn api_request<R>(&self, method: Method, path: &str) -> Result<HttpRequest<R, GoogleDriveApiError>, GoogleDriveError>
        where R: de::DeserializeOwned + 'static
    {
        Ok(HttpRequest::new_json(
            method, API_ENDPOINT.to_owned() + path,
            Duration::from_secs(API_REQUEST_TIMEOUT))
            .with_header(Authorization(Bearer {token: self.access_token()?}), false))
    }

    fn start_file_upload(&self, path: &str, mime_type: &str) -> GenericResult<String> {
        let (parent_id, name) = self.new_file(path)?;

        #[derive(Serialize)]
        struct Request<'a> {
            name: &'a str,
            #[serde(rename = "mimeType")]
            mime_type: &'a str,
            parents: Vec<String>,
        }

        let request = HttpRequest::new(
            Method::Post, UPLOAD_ENDPOINT.to_owned() + "/files?uploadType=resumable",
            Duration::from_secs(API_REQUEST_TIMEOUT),
            RawResponseReader::new(), JsonErrorReader::<GoogleDriveApiError>::new())
            .with_header(Authorization(Bearer {token: self.access_token()?}), false)
            .with_json(&Request {
                name: &name,
                mime_type: DIRECTORY_MIME_TYPE,
                parents: vec![parent_id],
            })?;

        let location = match self.client.send(request)?.headers.get::<Location>() {
            Some(location) => location.to_string(),
            None => return Err!(concat!(
                "Got an invalid response from Google Drive API: ",
                "upload session has been created, but session URI hasn't been returned"
            )),
        };

        Ok(location)
    }

    fn file_upload_request(&self, location: String, timeout: u64) -> HttpRequest<GoogleDriveFile, GoogleDriveApiError> {
        HttpRequest::new_json(Method::Put, location, Duration::from_secs(timeout))
    }

    // FIXME
    fn content_request<I, B, O>(&self, path: &str, request: &I, body: B) -> Result<O, HttpClientError<GoogleDriveApiError>>
        where I: ser::Serialize,
              B: Into<Body>,
              O: de::DeserializeOwned,
    {
        let url = UPLOAD_ENDPOINT.to_owned() + path;
        let mut headers = Headers::new();
        unreachable!();
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
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>> {
        let file = match self.stat_path(path)? {
            Some(file) => file,
            None => return Ok(None),
        };

        if file.type_() != FileType::Directory {
            return Err!("{:?} is not a directory", path);
        }

        let mut files = Vec::new();
        let mut children = self.list_children(&file.id)?;

        for (name, mut children) in children.drain() {
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                return Err!("{:?} directory contains a file with an invalid name: {:?}",
                            path, file.name)
            }

            if children.len() > 1 {
                return Err!("{:?} directory has {} files with {:?} name",
                            path, children.len(), name);
            }

            files.extend(children.drain(..).map(|file| File {
                type_: file.type_(),
                name: file.name,
            }));
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
        let location = self.start_file_upload(path, DIRECTORY_MIME_TYPE)?;
        let request = self.file_upload_request(location.to_string(), API_REQUEST_TIMEOUT)
            .with_text_body(ContentType(Mime::from_str(DIRECTORY_MIME_TYPE).unwrap()), "")?; // FIXME: unwrap
        self.client.send(request)?;
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

        unimplemented!();
//        let _: EmptyResponse = self.oauth_request("/files/delete_v2", &Request {
//            path: path
//        })?;

        Ok(())
    }
}

const DIRECTORY_MIME_TYPE: &'static str = "application/vnd.google-apps.folder";

#[derive(Deserialize, Clone)]
struct GoogleDriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

impl GoogleDriveFile {
    fn type_(&self) -> FileType {
        if self.mime_type == DIRECTORY_MIME_TYPE {
            FileType::Directory
        } else if self.mime_type.starts_with("application/vnd.google-apps.") {
            FileType::Other
        } else {
            FileType::File
        }
    }
}

// FIXME: Do we need it?
#[derive(Debug)]
pub enum GoogleDriveError {
    Oauth(String),
    Api(HttpClientError<GoogleDriveApiError>),
}

impl Error for GoogleDriveError {
    fn description(&self) -> &str {
        match *self {
            GoogleDriveError::Oauth(_) => "Google OAuth error",
            GoogleDriveError::Api(ref e) => e.description(),
        }
    }
}

impl fmt::Display for GoogleDriveError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            GoogleDriveError::Oauth(ref e) => write!(f, "{}", e),
            GoogleDriveError::Api(ref e) => e.fmt(f),
        }
    }
}

impl From<HttpClientError<GoogleDriveApiError>> for GoogleDriveError {
    fn from(e: HttpClientError<GoogleDriveApiError>) -> GoogleDriveError {
        GoogleDriveError::Api(e)
    }
}


#[derive(Debug, Deserialize)]
struct GoogleDriveApiError {
    error: GoogleDriveApiErrorObject,
}

#[derive(Debug, Deserialize)]
struct GoogleDriveApiErrorObject {
    message: String,
}

impl Error for GoogleDriveApiError {
    fn description(&self) -> &str {
        "Google Drive error"
    }
}

impl fmt::Display for GoogleDriveApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.description(), self.error.message)
    }
}