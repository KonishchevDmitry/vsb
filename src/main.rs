extern crate ansi_term;
extern crate atty;
extern crate chrono;
extern crate fern;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
#[macro_use]extern crate log;
extern crate mime;
extern crate serde;
#[macro_use]extern crate serde_derive;
extern crate serde_json;
extern crate tokio_core;

mod core;
mod logging;
mod providers;

fn main() {
    logging::init().expect("Failed to initialize the logging");
    providers::dropbox::Dropbox::new();
    info!("ok")
}
