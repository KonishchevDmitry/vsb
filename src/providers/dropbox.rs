use std::env;
use std::io::{self, Write};

use core::EmptyResult;

use futures::{Future, Stream};
use hyper::{Client, Method, Request};
use hyper::header::{UserAgent, Authorization, Bearer, ContentLength, ContentType};
use hyper_tls::HttpsConnector;
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
        let handle = self.core.handle();

        let client = Client::configure()
            .connector(HttpsConnector::new(4, &handle)?)
            .build(&handle);

        let uri = "https://api.dropboxapi.com/2/files/list_folder".parse()?;

        let json = r#"{"path":""}"#;

        let mut request = Request::new(Method::Post, uri);
        request.headers_mut().set(UserAgent::new("pyvsb-to-cloud"));
        request.headers_mut().set(Authorization(Bearer {
            token: self.access_token.to_owned(),
        }));
        request.headers_mut().set(ContentType::json());
        request.headers_mut().set(ContentLength(json.len() as u64));
        request.set_body(json);

        let post = client.request(request).and_then(|res| {
            println!("POST: {}", res.status());
            res.body().concat2()
        });

//        let work = client.get(uri).and_then(|res| {
//            println!("Response: {}", res.status());
//
//            res.body().for_each(|chunk| {
//                io::stdout()
//                    .write_all(&chunk)
//                    .map(|_| ())
//                    .map_err(From::from)
//            })
//        });

        let result = self.core.run(post);

        println!(">>> {}", String::from_utf8(result.unwrap().to_vec())?);

        Ok(())
    }
}