extern crate bytes;
extern crate bzip2;
extern crate chrono;
extern crate clap;
extern crate digest;
extern crate easy_logging;
extern crate hyper;
#[macro_use] extern crate lazy_static;
extern crate libc;
#[macro_use] extern crate log;
extern crate md5;
extern crate mime;
extern crate nix;
extern crate regex;
extern crate reqwest;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate serde_urlencoded;
extern crate serde_yaml;
extern crate sha2;
extern crate shellexpand;
extern crate tar;

use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::process;

use nix::errno::Errno;
use nix::fcntl::{self, FlockArg};

mod check;
#[macro_use] mod core;
mod config;
mod encryptor;
mod hash;
mod http_client;
mod provider;
mod providers;
mod storage;
mod stream_splitter;
mod sync;
mod util;

use core::{EmptyResult, GenericResult};
use easy_logging::GlobalContext;
use providers::dropbox::Dropbox;
use providers::filesystem::Filesystem;
use providers::google_drive::GoogleDrive;
use storage::{Storage, BackupGroups};

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

        if let Err(err) = sync_backups(backup) {
            error!("Sync failed: {}.", err);
            exit_code = 1;
        }
    }

    Ok(exit_code)
}

fn acquire_lock(config_path: &str) -> GenericResult<File> {
    let file = File::open(config_path).map_err(|e| format!(
        "Unable to open {:?}: {}", config_path, e))?;

    fcntl::flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock).map_err(|e| {
        if let nix::Error::Sys(Errno::EAGAIN) = e {
            format!(concat!(
                "Unable to exclusively run the program for {:?} configuration file: ",
                "it's already locked by another process"), config_path)
        } else {
            format!("Unable to flock() {:?}: {}", config_path, e)
        }
    })?;

    Ok(file)
}

fn sync_backups(backup_config: &config::Backup) -> EmptyResult {
    let local_storage = Storage::new_read_only(Filesystem::new(), &backup_config.src);
    let (local_backup_groups, local_ok) = get_backup_groups(&local_storage)?;
    check::check_backups(&local_storage, &local_backup_groups,
                         local_ok, backup_config.max_time_without_backups);

    let mut cloud_storage = match backup_config.provider {
        config::Provider::Dropbox {ref access_token} =>
            Storage::new(Dropbox::new(&access_token)?, &backup_config.dst),
        config::Provider::GoogleDrive {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new(GoogleDrive::new(&client_id, &client_secret, &refresh_token), &backup_config.dst),
    };
    let (cloud_backup_groups, cloud_ok) = get_backup_groups(&cloud_storage)?;

    info!("Syncing...");
    let sync_ok = sync::sync_backups(
        &local_storage, &local_backup_groups,
        &mut cloud_storage, &cloud_backup_groups, local_ok && cloud_ok,
        backup_config.max_backup_groups, &backup_config.encryption_passphrase);

    let (cloud_backup_groups, cloud_ok) = match get_backup_groups(&cloud_storage) {
        Ok(result) => result,
        Err(err) => {
            error!("Unable to check backups on {}: {}.", cloud_storage.name(), err);
            return Ok(());
        },
    };
    check::check_backups(&cloud_storage, &cloud_backup_groups,
                         sync_ok && cloud_ok, backup_config.max_time_without_backups);

    Ok(())
}

fn get_backup_groups(storage: &Storage) -> GenericResult<(BackupGroups, bool)> {
    info!("Checking backups on {}...", storage.name());
    let (backup_groups, ok) = storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", storage.name(), e))?;

    if log_enabled!(log::Level::Debug) {
        if backup_groups.is_empty() {
            debug!("There are no backup groups on {}.", storage.name());
        } else {
            debug!("Backup groups on {}:", storage.name());
            for (group_name, backups) in backup_groups.iter() {
                let backup_names = backups.iter().cloned().collect::<Vec<String>>().join(", ");
                debug!("{}: {}", group_name, backup_names);
            }
        }
    }

    Ok((backup_groups, ok))
}