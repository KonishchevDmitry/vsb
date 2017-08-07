use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Add;
use std::str::FromStr;
use std::time::Duration;

use hyper::header::{Authorization, Bearer, Location, ContentType};
use mime::Mime;
use serde::de;

use core::{EmptyResult, GenericResult};
use hash::{Hasher, Md5};
use http_client::{HttpClient, Method, HttpRequest, HttpResponse, EmptyRequest, RawResponseReader,
                  JsonErrorReader, HttpClientError};
use provider::{Provider, ProviderType, ReadProvider, WriteProvider, File, FileType};
use stream_splitter::{ChunkStreamReceiver, ChunkStream};

mod oauth;
use self::oauth::GoogleOauth;

const API_ENDPOINT: &'static str = "https://www.googleapis.com/drive/v3";
const API_REQUEST_TIMEOUT: u64 = 5;

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

    fn start_file_upload(&self, path: &str, mime_type: &ContentType, overwrite: bool) -> GenericResult<String> {
        let (parent_id, name, file_id) = self.get_new_file_info(path)?;
        if file_id.is_some() && !overwrite {
            return Err!("File already exists");
        }

        let method = match file_id {
            Some(_) => Method::Patch,
            None => Method::Post,
        };

        let mut url = UPLOAD_ENDPOINT.to_owned() + "/files";
        if let Some(ref file_id) = file_id {
            url = url + "/" + &file_id;
        }
        url += "?uploadType=resumable";

        let mut request = HttpRequest::new(
            method, url, Duration::from_secs(API_REQUEST_TIMEOUT),
            RawResponseReader::new(), JsonErrorReader::<GoogleDriveApiError>::new())
            .with_header(Authorization(Bearer {token: self.access_token()?}), false);

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
                mime_type: mime_type.as_ref(),
                parents: vec![parent_id],
            })?
        };

        let upload_url = match self.client.send(request)?.headers.get::<Location>() {
            Some(location) => location.to_string(),
            None => return Err!(concat!(
                "Got an invalid response from Google Drive API: ",
                "upload session has been created, but session URI hasn't been returned"
            )),
        };

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
            let file_metadata = self.client.send(self.api_request(Method::Get, &request_path)?)?;
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

    fn get_file_metadata(&self, id: &str) {

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

    fn delete_request(&self, path: &str) -> Result<HttpRequest<HttpResponse, GoogleDriveApiError>, GoogleDriveError> {
        Ok(HttpRequest::new(
            Method::Delete, API_ENDPOINT.to_owned() + path,
            Duration::from_secs(API_REQUEST_TIMEOUT),
            RawResponseReader::new(), JsonErrorReader::new())
            .with_header(Authorization(Bearer {token: self.access_token()?}), false))
    }

    fn file_upload_request(&self, location: String, timeout: u64) -> HttpRequest<GoogleDriveFile, GoogleDriveApiError> {
        HttpRequest::new_json(Method::Put, location, Duration::from_secs(timeout))
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
        Box::new(Md5::new())
    }

    fn max_request_size(&self) -> Option<u64> {
        None
    }

    fn create_directory(&self, path: &str) -> EmptyResult {
        let content_type = ContentType(Mime::from_str(DIRECTORY_MIME_TYPE).unwrap());
        let upload_url = self.start_file_upload(path, &content_type, false)?;
        let request = self.file_upload_request(upload_url, API_REQUEST_TIMEOUT)
            .with_text_body(content_type, "")?;
        self.client.send(request)?;
        Ok(())
    }

    // FIXME
    fn upload_file(&self, temp_path: &str, path: &str, chunk_streams: ChunkStreamReceiver) -> EmptyResult {
        let temp_path_components: Vec<&str> = temp_path.rsplitn(2, '/').collect();
        let path_components: Vec<&str> = path.rsplitn(2, '/').collect();

        if temp_path_components.len() != 2 {
            return Err!("Invalid path: {:?}", temp_path);
        }

        if path_components.len() != 2 {
            return Err!("Invalid path: {:?}", path);
        }

        if temp_path_components[1] != path_components[1] {
            return Err!("Temporary file must be in the same directory as destination file")
        }
        let name = path_components[0];

        let mut file = None;

        for result in chunk_streams.iter() {
            match result {
                Ok(ChunkStream::Stream(offset, chunk_stream)) => {
                    let content_type = ContentType::octet_stream();
                    let upload_url = self.start_file_upload(temp_path, &content_type, true)?;
                    let request = self.file_upload_request(upload_url, UPLOAD_REQUEST_TIMEOUT)
                        .with_body(content_type, None, chunk_stream)?;
                    file = Some(self.client.send(request)?);
                },
                Ok(ChunkStream::EofWithCheckSum(size, checksum)) => {
                    let file = file.unwrap();

                    let request = self.api_request(
                        Method::Get, &"/files/".to_owned().add(&file.id).add("?fields=md5Checksum"))?;
                    let metadata: GoogleDriveFileMetadata = self.client.send(request)?;

                    if metadata.md5_checksum != checksum {
                        if let Err(err) = self.delete(temp_path) {
                            error!("Failed to delete a temporary {:?} file from {}: {}.",
                                temp_path, self.name(), err);
                        }
                        return Err("Checksum mismatch".into());
                    }

                    #[derive(Serialize)]
                    struct RenameRequest<'a> {
                        name: &'a str,
                    }
                    let request = self.api_request(Method::Patch, &"/files/".to_owned().add(&file.id))?
                        .with_json(&RenameRequest {
                            name: name,
                        })?;
                    let _: GoogleDriveFile = self.client.send(request)?;

                    return Ok(())
                }
                Err(err) => return Err(err.into()),
            }
        }

        Err!("Chunk stream sender has been closed without a termination message")
    }

    // FIXME
    fn delete(&self, path: &str) -> EmptyResult {
        let file = self.stat_path(path)?;
        let file = file.unwrap();

        let request = self.delete_request(&"/files/".to_owned().add(&file.id))?;
        self.client.send(request)?;

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

#[derive(Deserialize)]
struct GoogleDriveFileMetadata {
    #[serde(rename = "md5Checksum")]
    md5_checksum: String,
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

// FIXME: Do we need it?
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