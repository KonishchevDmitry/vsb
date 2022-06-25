use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::ops::Add;
use std::time::{Duration, Instant};

use log::error;
use reqwest::StatusCode;
use serde::{ser, de};
use serde_derive::{Serialize, Deserialize};

use crate::core::{EmptyResult, GenericResult};
use crate::http_client::{
    HttpClient, HttpRequest, Method, HttpResponse, EmptyResponse, HttpClientError,
    ResponseReader, RawResponseReader, JsonReplyReader, JsonErrorReader};
use crate::util::hash::{Hasher, Hash, Md5};
use crate::util::stream_splitter::{ChunkStreamReceiver, ChunkStream};

use super::{Provider, ProviderType, ReadProvider, WriteProvider, UploadProvider, File, FileType};
use super::oauth::OauthClient;

const OAUTH_ENDPOINT: &str = "https://oauth.yandex.ru";
const API_ENDPOINT: &str = "https://cloud-api.yandex.net/v1/disk";

const API_REQUEST_TIMEOUT: u64 = 15;
const OPERATION_TIMEOUT: u64 = 60;
const UPLOAD_REQUEST_TIMEOUT: u64 = 60 * 60;

pub struct YandexDisk {
    oauth: OauthClient,
    client: HttpClient,
}

impl YandexDisk {
    pub fn new(client_id: &str, client_secret: &str, refresh_token: &str) -> GenericResult<YandexDisk> {
        Ok(YandexDisk {
            oauth: OauthClient::new(OAUTH_ENDPOINT, client_id, client_secret, refresh_token),
            client: HttpClient::new(),
        })
    }

    fn rename_file(&self, src: &str, dst: &str, overwrite: bool) -> EmptyResult {
        #[derive(Serialize)]
        struct Request {
            from: String,
            path: String,
            overwrite: bool,
        }

        let response: HttpResponse = self.raw_api_request(Method::POST, "/resources/move", &Request {
            from: disk_path(src),
            path: disk_path(dst),
            overwrite,
        })?;

        if response.status == StatusCode::CREATED {
            return Ok(());
        }

        #[derive(Deserialize)]
        struct Response {
            href: String,
        }

        let response: Response = JsonReplyReader::new().read(response)?;
        self.wait_operation(&response.href)
    }

    fn finish_upload(&self, temp_path: &str, path: &str, checksum: Hash) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            path: String,
            fields: &'a str,
        }

        #[derive(Deserialize)]
        struct Response {
            md5: String,
        }

        let response: Response = self.api_request(Method::GET, "/resources", &Request {
            path: disk_path(temp_path),
            fields: "md5",
        })?;

        if response.md5 != checksum.to_string() {
            if let Err(err) = self.delete(temp_path) {
                error!("Failed to delete a temporary {:?} file from {}: {}.",
                    temp_path, self.name(), err);
            }
            return Err!("Checksum mismatch");
        }

        self.rename_file(temp_path, path, false).map_err(|err| {
            if let Err(err) = self.delete(temp_path) {
                error!("Failed to delete a temporary {:?} file from {}: {}.",
                    temp_path, self.name(), err);
            }
            err
        })
    }

    fn wait_operation(&self, url: &str) -> EmptyResult {
        let deadline = Instant::now().add(Duration::from_secs(OPERATION_TIMEOUT));

        #[derive(Deserialize)]
        struct Response {
            status: String,
        }

        loop {
            let response: Response = self.send_request(HttpRequest::new_json(
                Method::GET, url.to_owned(), Duration::from_secs(API_REQUEST_TIMEOUT)
            ))?;

            match response.status.as_str() {
                "success" => return Ok(()),
                "in-progress" => {},
                _ => return Err!("{} operation has failed", self.name()),
            }

            let sleep_time = Duration::from_millis(500);

            let time_left = deadline.saturating_duration_since(Instant::now());
            if time_left < sleep_time {
                return Err!("{} operation has timed out", self.name());
            }

            std::thread::sleep(sleep_time);
        }
    }

    fn api_request<I, O>(&self, method: Method, path: &str, request: &I) -> Result<O, HttpClientError<ApiError>>
        where I: ser::Serialize,
              O: de::DeserializeOwned,
    {
        self.send_request(HttpRequest::new_json(
            method, api_url(path), Duration::from_secs(API_REQUEST_TIMEOUT)
        ).with_params(request)?)
    }

    fn raw_api_request<I>(&self, method: Method, path: &str, request: &I) -> Result<HttpResponse, HttpClientError<ApiError>>
        where I: ser::Serialize,
    {
        self.send_request(HttpRequest::new(
            method, api_url(path), Duration::from_secs(API_REQUEST_TIMEOUT),
            RawResponseReader::new(), JsonErrorReader::new()
        ).with_params(request)?)
    }

    fn send_request<O>(&self, request: HttpRequest<O, ApiError>) -> Result<O, HttpClientError<ApiError>> {
        let request = self.oauth.authenticate(request, "OAuth").map_err(|e|
            HttpClientError::Generic(e.to_string()))?;
        self.client.send(request)
    }
}

impl Provider for YandexDisk {
    fn name(&self) -> &'static str {
        "Yandex Disk"
    }

    fn type_(&self) -> ProviderType {
        ProviderType::Cloud
    }
}

impl ReadProvider for YandexDisk {
    fn list_directory(&self, path: &str) -> GenericResult<Option<Vec<File>>> {
        #[derive(Serialize)]
        struct Request<'a> {
            path: String,
            fields: &'a str,
            offset: usize,
        }

        #[derive(Deserialize)]
        struct Response {
            #[serde(rename="type")]
            type_: String,
            #[serde(rename="_embedded")]
            embedded: Option<Embedded>,
        }

        #[derive(Deserialize)]
        struct Embedded {
            items: Vec<Item>,
            total: usize,
        }

        #[derive(Deserialize)]
        struct Item {
            #[serde(rename="type")]
            type_: String,
            name: String,
            size: Option<u64>,
        }

        let mut offset: usize = 0;
        let mut files = HashMap::new();

        loop {
            let response = self.api_request(Method::GET, "/resources", &Request {
                path: disk_path(path),
                offset,
                fields: "type,_embedded.items.type,_embedded.items.name,_embedded.items.size,_embedded.offset,_embedded.total",
            });

            if let Err(HttpClientError::Api(ref e)) = response {
                if e.error == "DiskNotFoundError" {
                    return Ok(None);
                }
            }

            let response: Response = response?;
            let embedded = match response.embedded {
                Some(embedded) if response.type_ == "dir" => embedded,
                _ => return Err!("{:?} is not a directory", path),
            };

            offset += embedded.items.len();

            for item in embedded.items {
                files.insert(item.name.clone(), File {
                    name: item.name,
                    type_: match item.type_.as_str() {
                        "dir" => FileType::Directory,
                        "file" => FileType::File,
                        _ => FileType::Other,
                    },
                    size: item.size,
                });
            }

            if offset >= embedded.total {
                break;
            }
        }

        Ok(Some(files.into_values().collect()))
    }
}

impl WriteProvider for YandexDisk {
    fn create_directory(&self, path: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request {
            path: String,
        }

        let _: EmptyResponse = self.api_request(Method::PUT, "/resources", &Request {
            path: disk_path(path),
        })?;

        Ok(())
    }

    fn delete(&self, path: &str) -> EmptyResult {
        #[derive(Serialize)]
        struct Request {
            path: String,
        }

        let response = self.raw_api_request(Method::DELETE, "/resources", &Request {
            path: disk_path(path),
        })?;

        if response.status == StatusCode::NO_CONTENT {
            return Ok(());
        }

        #[derive(Deserialize)]
        struct Response {
            href: String,
        }

        let response: Response = JsonReplyReader::new().read(response)?;
        self.wait_operation(&response.href)
    }
}

impl UploadProvider for YandexDisk {
    fn hasher(&self) -> Box<dyn Hasher> {
        Box::new(Md5::new())
    }

    fn max_request_size(&self) -> Option<u64> {
        None
    }

    fn upload_file(
        &self, directory_path: &str, temp_name: &str, name: &str, chunk_streams: ChunkStreamReceiver,
    ) -> EmptyResult {
        let temp_path = format!("{}/{}", directory_path.trim_end_matches('/'), temp_name);
        let path = format!("{}/{}", directory_path.trim_end_matches('/'), name);

        #[derive(Serialize)]
        struct Request {
            path: String,
            overwrite: bool,
        }

        #[derive(Deserialize)]
        struct Response {
            operation_id: String,
            href: String,
        }

        let response: Response = self.api_request(Method::GET, "/resources/upload", &Request {
            path: disk_path(&temp_path),
            overwrite: true,
        })?;

        let operation_url = api_url(&format!("/operations/{}", response.operation_id));
        let upload_url = response.href;

        for (index, result) in chunk_streams.iter().enumerate() {
            match result {
                Ok(ChunkStream::Stream(offset, chunk_stream)) => {
                    assert_eq!(index, 0);
                    assert_eq!(offset, 0);

                    let request = HttpRequest::<HttpResponse, ApiError>::new(
                        Method::PUT, upload_url.clone(), Duration::from_secs(UPLOAD_REQUEST_TIMEOUT),
                        RawResponseReader::new(), JsonErrorReader::new(),
                    ).with_body("application/octet-stream", chunk_stream)?;

                    self.client.send(request)?;
                },

                Ok(ChunkStream::EofWithCheckSum(_size, checksum)) => {
                    self.wait_operation(&operation_url)?;
                    self.finish_upload(&temp_path, &path, checksum)?;
                    return Ok(())
                },

                Err(err) => {
                    return Err(err.into())
                },
            }
        }

        Err!("Chunk stream sender has been closed without a termination message")
    }
}

fn api_url(path: &str) -> String {
    format!("{}{}", API_ENDPOINT, path)
}

// Without this prefix Yandex Disk improperly handles paths with colon
fn disk_path(path: &str) -> String {
    format!("disk:{}", path)
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: String,
    message: String,
}

impl Error for ApiError {
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Yandex Disk error: {}", self.message.trim_end_matches('.'))
    }
}