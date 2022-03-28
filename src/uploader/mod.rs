use std::fs::File;
use std::os::unix::io::AsRawFd;

use easy_logging::GlobalContext;
use log::{log_enabled, debug, info, error};
use nix::errno::Errno;
use nix::fcntl::{self, FlockArg};

use crate::check;
use crate::config::{Config, BackupConfig, ProviderConfig};
use crate::core::{EmptyResult, GenericResult};
use crate::metrics;
use crate::providers::dropbox::Dropbox;
use crate::providers::filesystem::Filesystem;
use crate::providers::google_drive::GoogleDrive;
use crate::storage::{Storage, BackupGroup};
use crate::sync;

pub fn upload(config: &Config) -> GenericResult<bool> {
    let mut ok = true;
    let _lock = acquire_lock(&config.path)?;

    for backup in config.backups.iter() {
        let _context = GlobalContext::new(&backup.name);

        if let Err(err) = sync_backups(backup) {
            error!("Sync failed: {}.", err);
            ok = false;
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

fn sync_backups(backup_config: &BackupConfig) -> EmptyResult {
    // FIXME(konishchev): Support
    let upload_config = backup_config.upload.as_ref().unwrap();

    let local_storage = Storage::new_read_only(Filesystem::new(), &upload_config.src);
    let (local_backup_groups, local_ok) = get_backup_groups(&local_storage, true)?;
    check::check_backups(&local_storage, &local_backup_groups,
                         local_ok, upload_config.max_time_without_backups);

    if let Err(err) = metrics::collect(&backup_config.name, &local_backup_groups) {
        error!("Failed to collect metrics: {}.", err);
    }

    let cloud_storage = match upload_config.provider {
        ProviderConfig::Dropbox {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new(Dropbox::new(client_id, client_secret, refresh_token)?, &upload_config.dst),
        ProviderConfig::GoogleDrive {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new(GoogleDrive::new(client_id, client_secret, refresh_token), &upload_config.dst),
    };
    let (cloud_backup_groups, cloud_ok) = get_backup_groups(&cloud_storage, false)?;

    info!("Syncing...");
    let sync_ok = sync::sync_backups(
        &local_storage, &local_backup_groups,
        &cloud_storage, &cloud_backup_groups, local_ok && cloud_ok,
        upload_config.max_backup_groups, &upload_config.encryption_passphrase);

    let (cloud_backup_groups, cloud_ok) = match get_backup_groups(&cloud_storage, false) {
        Ok(result) => result,
        Err(err) => {
            error!("Unable to check backups on {}: {}.", cloud_storage.name(), err);
            return Ok(());
        },
    };
    check::check_backups(&cloud_storage, &cloud_backup_groups,
                         sync_ok && cloud_ok, upload_config.max_time_without_backups);

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