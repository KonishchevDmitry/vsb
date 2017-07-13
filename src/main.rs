extern crate ansi_term;
extern crate atty;
extern crate bytes;
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
extern crate sha2;
extern crate shellexpand;
extern crate tar;
extern crate tokio_core;

use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::process;

use nix::fcntl::{self, FlockArg};

#[macro_use] mod core;
mod config;
mod encryptor;
mod hash;
mod http_client;
mod logging;
mod provider;
mod providers;
mod storage;
mod stream_splitter;
mod sync;
mod util;

use core::{EmptyResult, GenericResult};
use logging::GlobalContext;
use providers::dropbox::Dropbox;
use providers::filesystem::Filesystem;
use storage::Storage;

fn main() {
    process::exit(match run(){
        Ok(exit_code) => exit_code,
        Err(err) => {
            error!("{}.", err);
            1
        }
    });
}

fn run() -> GenericResult<i32> {
    let config = config::load();
    let _lock = acquire_lock(&config.path)?;

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

    Ok(exit_code)
}

fn acquire_lock(path: &str) -> GenericResult<File> {
    let file = File::open(path).map_err(|e| format!("Unable to open {:?}: {}", path, e))?;

    fcntl::flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock).map_err(|e| {
        if let nix::Error::Sys(nix::Errno::EAGAIN) = e {
            format!(concat!(
                "Unable to exclusively run the program for {:?} configuration file: ",
                "it's already locked by another process"), path)
        } else {
            format!("Unable to flock() {:?}: {}", path, e)
        }
    })?;

    Ok(file)
}

fn sync_backups(backup_config: &config::Backup) -> EmptyResult {
    let local_storage = Storage::new_read_only(Filesystem::new(), &backup_config.src);

    let mut cloud_storage = match backup_config.provider {
        config::Provider::Dropbox {ref access_token} => Storage::new(
            Dropbox::new(&access_token)?, &backup_config.dst)
    };

    sync::sync_backups(&local_storage, &mut cloud_storage,
                       backup_config.max_backup_groups, &backup_config.encryption_passphrase)
}