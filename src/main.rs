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
extern crate serde_yaml;
extern crate shellexpand;
extern crate tar;
extern crate tokio_core;

use std::io::{self, Write};
use std::process;

#[macro_use] mod core;
mod config;
mod encryptor;
mod http_client;
mod logging;
mod provider;
mod providers;
mod storage;
mod sync;
mod util;

use config::Config;
use core::{EmptyResult, GenericResult};
use logging::GlobalContext;
use providers::dropbox::Dropbox;
use providers::filesystem::Filesystem;
use storage::Storage;

fn main() {
    let config = init().unwrap_or_else(|e| {
        let _ = writeln!(io::stderr(), "Error: {}.", e);
        process::exit(1);
    });

    let mut exit_code = 0;

    for backup in config.backups.iter() {
        let _context = GlobalContext::new(&backup.name);

        info!("Syncing...");
        if let Err(err) = sync_backups(backup) {
            error!("Sync failed: {}.", err);
            exit_code = 1;
        } else {
            info!("Sync completed.")
        }
    }

    process::exit(exit_code);
}

fn init() -> GenericResult<Config> {
    let config = config::load()?;
    logging::init().map_err(|e| format!("Failed to initialize the logging: {}", e))?;
    Ok(config)
}

fn sync_backups(backup_config: &config::Backup) -> EmptyResult {
    let local_storage = Storage::new_read_only(Filesystem::new(), &backup_config.src);

    let mut cloud_storage = match backup_config.provider {
        config::Provider::Dropbox {ref access_token} => Storage::new(
            Dropbox::new(&access_token)?, &backup_config.dst)
    };

    sync::sync_backups(&local_storage, &mut cloud_storage, backup_config.max_backup_groups)
}