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

#[macro_use] mod core;
mod http_client;
mod logging;
mod providers;

fn main() {
    logging::init().expect("Failed to initialize the logging");
    let provider = providers::dropbox::Dropbox::new().unwrap();
    if let Err(e) = provider.test() {
        error!("Request failed: {}.", e)
    } else {
        info!("ok")
    }
}
