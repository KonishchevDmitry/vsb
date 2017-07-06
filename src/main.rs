extern crate ansi_term;
extern crate atty;
extern crate chrono;
extern crate fern;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
#[macro_use] extern crate log;
extern crate mime;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate tokio_core;

use std::env;

#[macro_use] mod core;
mod encryptor;
mod http_client;
mod logging;
mod provider;
mod providers;

// FIXME
fn main() {
    logging::init().expect("Failed to initialize the logging");
    let dropbox = providers::dropbox::Dropbox::new(&env::var("DROPBOX_ACCESS_TOKEN")
        .expect("DROPBOX_ACCESS_TOKEN environment variable is not set")).unwrap();
    let provider: &provider::Provider = &dropbox as &provider::Provider;
//    encryptor::Encryptor::new().unwrap();
    provider.test();
}
