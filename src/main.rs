extern crate ansi_term;
extern crate atty;
extern crate chrono;
extern crate fern;
extern crate futures;
extern crate hyper;
extern crate hyper_tls;
#[macro_use]extern crate log;
extern crate tokio_core;

mod core;
mod logging;
mod providers;

fn main() {
    logging::init().expect("Failed to initialize the logging");
    providers::dropbox::Dropbox::new();
    info!("ok")
}
