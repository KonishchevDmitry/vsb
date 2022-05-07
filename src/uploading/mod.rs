mod check;
mod metrics;
mod sync;
mod config;

use std::fs::File;
use std::os::unix::io::AsRawFd;

use easy_logging::GlobalContext;
use log::{debug, error, info, log_enabled};
use nix::errno::Errno;
use nix::fcntl::{self, FlockArg};

use crate::config::Config;
use crate::core::{EmptyResult, GenericResult};
use crate::providers::dropbox::Dropbox;
use crate::providers::filesystem::Filesystem;
use crate::providers::google_drive::GoogleDrive;
use crate::storage::{BackupGroup, Storage};

pub use config::{UploadConfig, ProviderConfig};

pub fn upload(config: &Config) -> GenericResult<bool> {
    let mut ok = true;
    let _lock = acquire_lock(&config.path)?;

    for backup in &config.backups {
        if let Some(upload_config) = backup.upload.as_ref() {
            let _context = GlobalContext::new(&backup.name);

            if let Err(err) = sync_backups(&backup.name, &backup.path, upload_config) {
                error!("Sync failed: {}.", err);
                ok = false;
            }
        }
    }

    if let Some(path) = config.prometheus_metrics.as_ref() {
        if let Err(err) = metrics::save(path) {
            error!("Failed to save Prometheus metrics to {:?}: {}.", path, err);
            ok = false;
        }
    }

    Ok(ok)
}

// FIXME(konishchev): Use for also for backup
fn acquire_lock(config_path: &str) -> GenericResult<File> {
    let file = File::open(config_path).map_err(|e| format!(
        "Unable to open {:?}: {}", config_path, e))?;

    fcntl::flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock).map_err(|e| {
        if e == Errno::EAGAIN {
            format!(concat!(
                "Unable to exclusively run the program for {:?} configuration file: ",
                "it's already locked by another process",
            ), config_path)
        } else {
            format!("Unable to flock() {:?}: {}", config_path, e)
        }
    })?;

    Ok(file)
}

fn sync_backups(name: &str, path: &str, config: &UploadConfig) -> EmptyResult {
    let local_storage = Storage::new_read_only(Filesystem::new(), path);
    let (local_backup_groups, local_ok) = get_backup_groups(&local_storage, true)?;
    check::check_backups(&local_storage, &local_backup_groups,
                         local_ok, config.max_time_without_backups);

    if let Err(err) = metrics::collect(name, &local_backup_groups) {
        error!("Failed to collect metrics: {}.", err);
    }

    let cloud_storage = match config.provider {
        ProviderConfig::Dropbox {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new_upload(Dropbox::new(client_id, client_secret, refresh_token)?, &config.path),
        ProviderConfig::GoogleDrive {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new_upload(GoogleDrive::new(client_id, client_secret, refresh_token), &config.path),
    };
    let (cloud_backup_groups, cloud_ok) = get_backup_groups(&cloud_storage, false)?;

    info!("Syncing...");
    let sync_ok = sync::sync_backups(
        &local_storage, &local_backup_groups,
        &cloud_storage, &cloud_backup_groups, local_ok && cloud_ok,
        config.max_backup_groups, &config.encryption_passphrase);

    let (cloud_backup_groups, cloud_ok) = match get_backup_groups(&cloud_storage, false) {
        Ok(result) => result,
        Err(err) => {
            error!("Unable to check backups on {}: {}.", cloud_storage.name(), err);
            return Ok(());
        },
    };
    check::check_backups(&cloud_storage, &cloud_backup_groups,
                         sync_ok && cloud_ok, config.max_time_without_backups);

    Ok(())
}

fn get_backup_groups(storage: &Storage, verify: bool) -> GenericResult<(Vec<BackupGroup>, bool)> {
    info!("Checking backups on {}...", storage.name());
    let (groups, ok) = storage.get_backup_groups(verify).map_err(|e| format!(
        "Failed to list backup groups on {}: {}", storage.name(), e))?;

    if log_enabled!(log::Level::Debug) {
        if groups.is_empty() {
            debug!("There are no backup groups on {}.", storage.name());
        } else {
            debug!("Backup groups on {}:", storage.name());
            for group in &groups {
                let backup_names = group.backups.iter()
                    .map(|backup| backup.name.as_str())
                    .collect::<Vec<&str>>().join(", ");
                debug!("{}: {}", group.name, backup_names);
            }
        }
    }

    Ok((groups, ok))
}
