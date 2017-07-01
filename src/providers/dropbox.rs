use std::env;

use core::{EmptyResult, GenericResult};

use hyper::header::{Authorization, Bearer};

use http_client::{HttpClient, HttpClientError};

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

        let result = self.client.json_request::<Request, Response, DropboxApiError>(
            "https://api.dropboxapi.com/2/files/list_folder", &Request{path: "/invalid"});

        if let Err(HttpClientError::Api(ref e)) = result {
            error!(">>> {}", e.error.path.tag);
        }

        info!("Response: {:?}", result);

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct DropboxApiError {
    error: DropboxError,
    error_summary: String,
}

#[derive(Debug, Deserialize)]
struct DropboxError {
    path: PathError,
}

#[derive(Debug, Deserialize)]
struct PathError {
    #[serde(rename = ".tag")]
    tag: String,
}