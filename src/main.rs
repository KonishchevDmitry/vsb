extern crate ansi_term;
extern crate atty;
extern crate chrono;
extern crate fern;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
#[macro_use] extern crate lazy_static;
#[macro_use] extern crate log;
extern crate mime;
extern crate nix;
extern crate regex;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate tar;
extern crate tokio_core;

use std::env;

#[macro_use] mod core;
mod encryptor;
mod http_client;
mod logging;
mod provider;
mod providers;
mod storage;
mod uploader;
mod util;

// FIXME
fn main() {
    logging::init().expect("Failed to initialize the logging");
    let dropbox = providers::dropbox::Dropbox::new(&env::var("DROPBOX_ACCESS_TOKEN")
        .expect("DROPBOX_ACCESS_TOKEN environment variable is not set")).unwrap();
    let filesystem = providers::filesystem::Filesystem::new("/Users/konishchev/.backup");
//    encryptor::Encryptor::new().unwrap();
    let uploader = uploader::Uploader::new(storage::Storage::new_read_only(filesystem), storage::Storage::new(dropbox));
    uploader.test();
}
