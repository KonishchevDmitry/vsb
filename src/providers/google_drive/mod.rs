mod oauth;

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Add;
use std::time::Duration;

use serde::de;

use core::{EmptyResult, GenericResult};
use hash::{Hasher, Md5};
use http_client::{HttpClient, Method, HttpRequest, HttpResponse, EmptyRequest, RawResponseReader,
                  JsonErrorReader, HttpClientError, headers};
use provider::{Provider, ProviderType, ReadProvider, WriteProvider, File, FileType};
use stream_splitter::{ChunkStreamReceiver, ChunkStream};

use self::oauth::GoogleOauth;

const API_ENDPOINT: &'static str = "https://www.googleapis.com/drive/v3";
const API_REQUEST_TIMEOUT: u64 = 15;

const UPLOAD_ENDPOINT: &'static str = "https://www.googleapis.com/upload/drive/v3";
const UPLOAD_REQUEST_TIMEOUT: u64 = 60 * 60;

pub struct GoogleDrive {
    client: HttpClient,
    oauth: GoogleOauth,
}

impl GoogleDrive {
    pub fn new(client_id: &str, client_secret: &str, refresh_token: &str) -> GoogleDrive {
        GoogleDrive {
            client: HttpClient::new(),
            oauth: GoogleOauth::new(client_id, client_secret, refresh_token),
        }
    }

    fn start_file_upload(&self, path: &str, mime_type: &str, overwrite: bool) -> GenericResult<String> {
        let (parent_id, name, file_id) = self.get_new_file_info(path)?;
        if file_id.is_some() && !overwrite {
            return Err!("File already exists");
        }

        let method = match file_id {
            Some(_) => Method::PATCH,
            None => Method::POST,
        };

        let mut url = UPLOAD_ENDPOINT.to_owned() + "/files";
        if let Some(ref file_id) = file_id {
            url = url + "/" + &file_id;
        }
        url += "?uploadType=resumable";

        let mut request = self.authenticate(
            HttpRequest::new(
                method, url, Duration::from_secs(API_REQUEST_TIMEOUT),
                RawResponseReader::new(), JsonErrorReader::<GoogleDriveApiError>::new())
        )?;

        request = if file_id.is_some() {
            request.with_json(&EmptyRequest {})?
        } else {
            #[derive(Serialize)]
            struct Request<'a> {
                name: &'a str,
                #[serde(rename = "mimeType")]
                mime_type: &'a str,
                parents: Vec<String>,
            }

            request.with_json(&Request {
                name: &name,
                mime_type: mime_type,
                parents: vec![parent_id],
            })?
        };

        let upload_url = self.client.send(request)?
            .get_header(headers::LOCATION)
            .and_then(|location: Option<&str>| location.ok_or_else(||
                "Upload session has been created, but session URI hasn't been returned".into()))
            .map_err(|e| format!("Got an invalid response from Google Drive API: {}", e))?.to_owned();

        Ok(upload_url)
    }

    fn get_new_file_info(&self, path: &str) -> GenericResult<(String, String, Option<String>)> {
        if path == "/" {
            return Err!("An attempt to upload a file as {:?}", path)
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
        let files = self.list_children(&parent.id)?;

        let file_id = match get_file(files, path, &name)? {
            Some(file) => Some(file.id),
            None => None,
        };

        return Ok((parent.id, name, file_id))
    }

    fn stat_path(&self, path: &str) -> GenericResult<Option<GoogleDriveFile>> {
        let mut cur_path = "/".to_owned();
        let mut cur_dir_id = "root".to_owned();

        if path == "/" {
            let request_path = "/files/".to_owned() + &cur_dir_id;
            let file_metadata = self.client.send(self.api_request(Method::GET, &request_path)?)?;
            return Ok(Some(file_metadata));
        } else if !path.starts_with('/') || path.ends_with('/') {
            return Err!("Invalid path: {:?}", path);
        }

        let mut components = path.split('/');
        assert!(components.next().unwrap().is_empty());

        let mut component = components.next().unwrap();

        loop {
            let files = self.list_children(&cur_dir_id).map_err(|e| format!(
                "Error while reading {:?} directory: {}", cur_path, e))?;

            if !cur_path.ends_with('/') {
                cur_path.push('/');
            }
            cur_path += component;

            let file = match get_file(files, &cur_path, &component)? {
                Some(file) => file,
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
            let request = self.api_request(Method::GET, "/files")?.with_params(&request_params)?;
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

    fn delete_file(&self, path: &str, only_if_exists: bool) -> EmptyResult {
        let file = match self.stat_path(path)? {
            Some(file) => file,
            None => {
                if only_if_exists {
                    return Ok(())
                } else {
                    return Err!("No such file or directory")
                }
            },
        };

        let request = self.delete_request(&"/files/".to_owned().add(&file.id))?;
        self.client.send(request)?;

        Ok(())
    }

    fn authenticate<'a, R, E>(&self, request: HttpRequest<'a, R, E>) -> Result<HttpRequest<'a, R, E>, GoogleDriveError> {
        let access_token = self.oauth.get_access_token(Duration::from_secs(API_REQUEST_TIMEOUT))
            .map_err(|e| GoogleDriveError::Oauth(format!(
                "Unable obtain a Google OAuth token: {}", e)))?;

        Ok(request.with_header(headers::AUTHORIZATION, format!("Bearer {}", access_token))
            .map_err(|_| GoogleDriveError::Oauth(s!("Got an invalid Google OAuth token")))?)
    }

    fn api_request<R>(&self, method: Method, path: &str) -> Result<HttpRequest<R, GoogleDriveApiError>, GoogleDriveError>
        where R: de::DeserializeOwned + 'static
    {
        Ok(self.authenticate(
            HttpRequest::new_json(
                method, API_ENDPOINT.to_owned() + path,
                Duration::from_secs(API_REQUEST_TIMEOUT))
        )?)
    }

    fn delete_request(&self, path: &str) -> Result<HttpRequest<HttpResponse, GoogleDriveApiError>, GoogleDriveError> {
        Ok(self.authenticate(
            HttpRequest::new(
                Method::DELETE, API_ENDPOINT.to_owned() + path,
                Duration::from_secs(API_REQUEST_TIMEOUT),
                RawResponseReader::new(), JsonErrorReader::new())
        )?)
    }

    fn file_upload_request(&self, location: String, timeout: u64) -> HttpRequest<GoogleDriveFile, GoogleDriveApiError> {
        HttpRequest::new_json(Method::PUT, location, Duration::from_secs(timeout))
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
    fn hasher(&self) -> Box<Hasher> {
        Box::new(Md5::new())
    }

    fn max_request_size(&self) -> Option<u64> {
        None
    }

    fn create_directory(&self, path: &str) -> EmptyResult {
        let content_type = DIRECTORY_MIME_TYPE;
        let upload_url = self.start_file_upload(path, content_type, false)?;
        let request = self.file_upload_request(upload_url, API_REQUEST_TIMEOUT)
            .with_text_body(content_type, "")?;
        self.client.send(request)?;
        Ok(())
    }

    fn upload_file(&self, directory_path: &str, temp_name: &str, name: &str,
                   chunk_streams: ChunkStreamReceiver) -> EmptyResult {
        let temp_path = directory_path.trim_right_matches('/').to_owned().add("/").add(temp_name);
        let mut file = None;

        for result in chunk_streams.iter() {
            match result {
                Ok(ChunkStream::Stream(offset, chunk_stream)) => {
                    assert!(file.is_none());
                    assert_eq!(offset, 0);

                    let content_type = "application/octet-stream";
                    let upload_url = self.start_file_upload(&temp_path, content_type, true)?;
                    let request = self.file_upload_request(upload_url, UPLOAD_REQUEST_TIMEOUT)
                        .with_body(content_type, chunk_stream)?;
                    file = Some(self.client.send(request)?);
                },
                Ok(ChunkStream::EofWithCheckSum(size, checksum)) => {
                    if size == 0 {
                        return Err!("An attempt to upload an empty file");
                    }

                    let file = file.unwrap();

                    #[derive(Deserialize)]
                    struct Metadata {
                        #[serde(rename = "md5Checksum")]
                        md5_checksum: String,
                    }

                    let request = self.api_request(
                        Method::GET, &"/files/".to_owned().add(&file.id).add("?fields=md5Checksum"))?;
                    let metadata: Metadata = self.client.send(request)?;

                    if metadata.md5_checksum != checksum {
                        if let Err(e) = self.delete_file(&temp_path, true) {
                            error!("Failed to delete a temporary {:?} file from {}: {}.",
                                   temp_path, self.name(), e);
                        }
                        return Err!("Checksum mismatch");
                    }

                    #[derive(Serialize)]
                    struct RenameRequest<'a> {
                        name: &'a str,
                    }
                    let request = self.api_request(
                        Method::PATCH, &"/files/".to_owned().add(&file.id))?
                        .with_json(&RenameRequest {
                            name: name,
                        })?;
                    let _: GoogleDriveFile = self.client.send(request)?;

                    return Ok(())
                }
                Err(err) => {
                    if file.is_some() {
                        if let Err(e) = self.delete_file(&temp_path, true) {
                            error!("Failed to delete a temporary {:?} file from {}: {}.",
                                   temp_path, self.name(), e);
                        }
                    }
                    return Err(err.into())
                },
            }
        }

        Err!("Chunk stream sender has been closed without a termination message")
    }

    fn delete(&self, path: &str) -> EmptyResult {
        self.delete_file(path, false)
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

fn get_file(mut directory_files: HashMap<String, Vec<GoogleDriveFile>>, path: &str, name: &str)
    -> GenericResult<Option<GoogleDriveFile>>
{
    Ok(match directory_files.remove(name) {
        Some(mut files) => {
            match files.len() {
                0 => unreachable!(),
                1 => Some(files.pop().unwrap()),
                _ => return Err!("{:?} path is unambiguous: it resolves to {} files",
                            path, files.len()),
            }
        },
        None => None,
    })
}

#[derive(Debug)]
enum GoogleDriveError {
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
        write!(f, "{}: {}", self.description(), self.error.message.trim_right_matches('.'))
    }
}
