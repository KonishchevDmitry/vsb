use std::env;

use core::{EmptyResult, GenericResult};

use hyper::header::{Authorization, Bearer};

use http_client::HttpClient;

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

        self.client.json_request("https://api.dropboxapi.com/2/files/list_folder", &Request{path: ""})?;

        Ok(())
    }
}