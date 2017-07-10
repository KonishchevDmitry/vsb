extern crate ansi_term;
extern crate atty;
extern crate chrono;
extern crate clap;
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
extern crate shellexpand;
extern crate tar;
extern crate tokio_core;

use std::env;
use std::io::{self, Write};
use std::process;

mod config;
#[macro_use] mod core;
mod encryptor;
mod http_client;
mod logging;
mod provider;
mod providers;
mod storage;
mod sync;
mod util;

use providers::dropbox::Dropbox;
use providers::filesystem::Filesystem;
use storage::Storage;

// FIXME
fn main() {
    config::load().unwrap_or_else(|e| {
        writeln!(io::stderr(), "Error: {}.", e);
        process::exit(1);
    });

    logging::init().expect("Failed to initialize the logging");
    return;

    let local_storage = Storage::new_read_only(Filesystem::new(), "/Users/konishchev/.backup");

    let mut cloud_storage = Storage::new(Dropbox::new(&env::var("DROPBOX_ACCESS_TOKEN")
        .expect("DROPBOX_ACCESS_TOKEN environment variable is not set")).unwrap(), "/Backups/macos.laptop");

    sync::sync_backups(&local_storage, &mut cloud_storage).map_err(|e| error!("{}.", e)).unwrap();
}
