mod check;
mod metrics;
mod sync;
mod config;

use easy_logging::GlobalContext;
use log::{debug, error, info, warn, log_enabled};

use crate::config::Config;
use crate::core::{EmptyResult, GenericResult};
use crate::providers::dropbox::Dropbox;
use crate::providers::filesystem::Filesystem;
use crate::providers::google_drive::GoogleDrive;
use crate::providers::yandex_disk::YandexDisk;
use crate::storage::{BackupGroup, Storage};
use crate::util::sys::acquire_lock;

pub use config::{UploadConfig, ProviderConfig};

pub fn upload(config: &Config, verify: bool) -> GenericResult<bool> {
    let mut ok = true;
    let _lock = acquire_lock(&config.path)?;

    let mut collect_metrics = true;
    let mut metrics_path = config.prometheus_metrics.as_ref();

    if !verify {
        collect_metrics = false;
        if metrics_path.is_some() {
            warn!("Skip metrics collection due to disabled backup verification.");
            metrics_path = None;
        }
    }

    for backup in &config.backups {
        if let Some(upload_config) = backup.upload.as_ref() {
            let _context = GlobalContext::new(&backup.name);

            if let Err(err) = sync_backups(&backup.name, &backup.path, upload_config, verify, collect_metrics) {
                error!("Sync failed: {}.", err);
                ok = false;
            }
        }
    }

    if let Some(path) = metrics_path {
        if let Err(err) = metrics::save(path) {
            error!("Failed to save Prometheus metrics to {:?}: {}.", path, err);
            ok = false;
        }
    }

    Ok(ok)
}

fn sync_backups(
    name: &str, path: &str, config: &UploadConfig, verify: bool, collect_metrics: bool,
) -> EmptyResult {
    let local_storage = Storage::new_read_only(Filesystem::new(), path);

    let (local_backup_groups, local_ok) = get_backup_groups(&local_storage, verify)?;
    check::check_backups(&local_storage, &local_backup_groups,
                         local_ok, config.max_time_without_backups);

    if collect_metrics {
        if let Err(err) = metrics::collect(name, &local_backup_groups) {
            error!("Failed to collect metrics: {}.", err);
        }
    }

    let cloud_storage = match config.provider {
        ProviderConfig::Dropbox {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new_upload(Dropbox::new(client_id, client_secret, refresh_token)?, &config.path),
        ProviderConfig::GoogleDrive {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new_upload(GoogleDrive::new(client_id, client_secret, refresh_token), &config.path),
        ProviderConfig::YandexDisk {ref client_id, ref client_secret, ref refresh_token} =>
            Storage::new_upload(YandexDisk::new(client_id, client_secret, refresh_token)?, &config.path),
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
