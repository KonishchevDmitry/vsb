use std::time::Duration;

use crate::storage::{Storage, BackupGroups};

pub fn check_backups(storage: &Storage, backup_groups: &BackupGroups, consistent: bool,
                     max_time_without_backups: Option<Duration>) {
    let mut last_backup = None;

    for (group_name, backups) in backup_groups.iter() {
        if backups.is_empty() {
            let error = format!("{} has an empty {:?} backup group.", storage.name(), group_name);
            if consistent {
                error!("{}", error);
            } else {
                warn!("{}", error);
            }
        } else {
            last_backup = Some(backups.iter().rev().next().unwrap());
        }
    }

    let last_backup = match last_backup {
        Some(last_backup) => last_backup,
        None => {
            error!("{} have no backups.", storage.name());
            return;
        }
    };

    let max_time_without_backups = match max_time_without_backups {
        Some(duration) => duration,
        None => return,
    };

    let last_backup_time = match storage.get_backup_time(&last_backup) {
        Ok(last_backup_time) => last_backup_time,
        Err(err) => {
            error!("Failed to determine a time when backup has been created: {}.", err);
            return;
        }
    };

    let time_from_last_backup = match last_backup_time.elapsed() {
        Ok(duration) => duration,
        Err(_) => {
            error!(concat!(
                "Failed to check last backup time: ",
                "the latest backup ({:?}) on {} has backup time in the future."),
                last_backup, storage.name());
            return;
        }
    };

    if time_from_last_backup < max_time_without_backups {
        return;
    }

    let minute_seconds = 60;
    let hour_seconds = 60 * minute_seconds;
    let day_seconds = 24 * hour_seconds;

    let mut human_durations = Vec::new();
    let mut elapsed_seconds = time_from_last_backup.as_secs();

    for &(unit_name, unit_seconds) in &[
        ("days", day_seconds),
        ("hours", hour_seconds),
        ("minutes", minute_seconds),
    ] {
        let units = elapsed_seconds / unit_seconds;
        if units != 0 {
            human_durations.push(format!("{} {}", units, unit_name));
            elapsed_seconds %= unit_seconds;
        }
    }

    error!("{} doesn't have any backup for last {}.", storage.name(), human_durations.join(" "));
}