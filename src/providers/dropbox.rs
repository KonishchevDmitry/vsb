use std::env;
use std::io::{self, Write};

use core::{EmptyResult, GenericError, GenericResult};

use futures::{self, Future, Stream};
use hyper::{Client, Method, Request, Response, Error};
use hyper::header::{UserAgent, Authorization, Bearer, ContentLength, ContentType};
use hyper_tls::HttpsConnector;
use mime;
use serde::ser;
use serde_json;
use tokio_core::reactor::Core;

pub struct Dropbox {
    access_token: String,
    core: Core,
}

impl Dropbox {
    pub fn new() {
        Dropbox {
            // FIXME
            access_token: env::var("DROPBOX_ACCESS_TOKEN").unwrap(),
            core: Core::new().unwrap(),
        }.test().unwrap();

    }

    fn test(&mut self) -> EmptyResult {
        #[derive(Serialize)]
        struct Request<'a> {
            path: &'a str,
        }

        self.json_request("/files/list_folder", &Request{path: ""})?;

        Ok(())
    }

    fn json_request<T: ser::Serialize>(&mut self, path: &str, request: &T) -> EmptyResult {
        let handle = self.core.handle();

        let client = Client::configure()
            .connector(HttpsConnector::new(4, &handle)?)
            .build(&handle);

        let uri = "https://api.dropboxapi.com/2/files/list_folder".parse()?;

        let json = serde_json::to_string(request)?;

        let mut http_request = Request::new(Method::Post, uri);
        http_request.headers_mut().set(UserAgent::new("pyvsb-to-cloud"));
        http_request.headers_mut().set(Authorization(Bearer {
            token: self.access_token.to_owned(),
        }));
        http_request.headers_mut().set(ContentType::json());
        http_request.headers_mut().set(ContentLength(json.len() as u64));
        http_request.set_body(json);

        let post = client.request(http_request).map_err(|e| -> GenericError { From::from(e.to_string()) }).and_then(|response: Response| {
            println!("POST: {}", response.status());
            {
                let content_type = response.headers().get::<ContentType>().unwrap();
                if content_type.type_() == mime::TEXT && content_type.subtype() == mime::PLAIN {
//                    panic!("some error");
                    return futures::future::err(From::from("some-error-occurred"));
                }
            }

            futures::future::ok(response)
        }).and_then(|response: Response| {
            response.body().concat2().map_err(|e| -> GenericError {From::from(e.to_string())})
        }).and_then(|body| {
            println!("> {}", String::from_utf8(body.to_vec()).unwrap());
            futures::future::ok(())
        });

        let result = self.core.run(post);

//        println!(">>> {}", String::from_utf8(result.unwrap().to_vec())?);
        println!(">>> {}", result.unwrap_err());

        Ok(())
    }
}