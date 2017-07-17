use log;

use core::EmptyResult;
use storage::{Storage, BackupGroups, Backups};

pub fn sync_backups(local_storage: &Storage, cloud_storage: &mut Storage,
                    max_backup_groups: usize, encryption_passphrase: &str) -> EmptyResult {
    let (local_groups, local_ok) = local_storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", local_storage.name(), e))?;

    let (cloud_groups, cloud_ok) = cloud_storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", cloud_storage.name(), e))?;

    if log_enabled!(log::LogLevel::Debug) {
        for &(storage, groups) in &[
            (local_storage, &local_groups),
            (cloud_storage, &cloud_groups)
        ] {
            if groups.is_empty() {
                debug!("There are no backup groups on {}.", storage.name());
            } else {
                debug!("Backup groups on {}:", storage.name());
                for (group_name, backups) in groups.iter() {
                    debug!("{}: {}", group_name, backups.iter().cloned().collect::<Vec<String>>().join(", "));
                }
            }
        }
    }

    let mut ok = local_ok && cloud_ok;

    if let Err(err) = check_backup_groups(&local_groups, &cloud_groups) {
        error!("{}.", err);
        ok = false;
    }

    // FIXME: Drop develop mode
    let develop_mode = if cfg!(debug_assertions) {
        error!("Attention! Running in develop mode.");
        ok = false;
        true
    } else {
        false
    };

    let target_groups = get_target_backup_groups(local_groups, &cloud_groups, max_backup_groups);
    let no_backups = Backups::new();

    for (group_name, target_backups) in target_groups.iter() {
        if target_backups.is_empty() {
            continue;
        }

        let cloud_backups = match cloud_groups.get(group_name) {
            Some(backups) => backups,
            None => {
                info!("Creating {:?} backup group on {}...", group_name, cloud_storage.name());

                if let Err(err) = cloud_storage.create_backup_group(group_name) {
                    error!("Failed to create {:?} backup group on {}: {}.",
                           group_name, cloud_storage.name(), err);
                    ok = false;
                    continue;
                }

                &no_backups
            },
        };

        let mut first = true;
        for backup_name in target_backups.iter() {
            if develop_mode && first {
                first = false;
                continue;
            }

            if cloud_backups.contains(backup_name) {
                continue;
            }

            let backup_path = local_storage.get_backup_path(group_name, backup_name, false);
            info!("Uploading {:?} backup to {}...", backup_path, cloud_storage.name());

            if let Err(err) = cloud_storage.upload_backup(
                &backup_path, group_name, backup_name, encryption_passphrase) {
                error!("Failed to upload {:?} backup to {}: {}.",
                       backup_path, cloud_storage.name(), err);
                ok = false;
            }
        }
    }

    for (group_name, _) in cloud_groups.iter() {
        if target_groups.contains_key(group_name) {
            continue
        }

        if !ok {
            warn!("Skipping deletion of {:?} backup group from {} because of the errors above.",
                  group_name, cloud_storage.name());
            continue;
        }

        // FIXME: Change to info after testing
        warn!("Deleting {:?} backup group from {}...", group_name, cloud_storage.name());
        if let Err(err) = cloud_storage.delete_backup_group(group_name) {
            error!("Failed to delete {:?} backup backup group from {}: {}.",
                   group_name, cloud_storage.name(), err)
        }
    }

    Ok(())
}

fn check_backup_groups(local_groups: &BackupGroups, cloud_groups: &BackupGroups) -> EmptyResult {
    let local_groups_num = local_groups.iter().filter(
        |&(_group_name, backups)| !backups.is_empty()).count();
    let cloud_groups_num = cloud_groups.len();

    if local_groups_num < 2 && cloud_groups_num > local_groups_num {
        return Err!(
            "A possible backup corruption: Cloud contains more backup groups than stored locally.")
    }

    Ok(())
}

fn get_target_backup_groups(local_groups: BackupGroups, cloud_groups: &BackupGroups, max_groups: usize) -> BackupGroups {
    let mut target_groups = local_groups;

    for (group_name, backups) in cloud_groups.iter() {
        target_groups.entry(group_name.clone()).or_insert_with(Backups::new).extend(
            backups.iter().cloned());
    }

    if target_groups.len() > max_groups {
        let mut groups_num = 0;
        let mut first_group_name = None;

        for (group_name, backups) in target_groups.iter().rev() {
            if backups.is_empty() {
                continue
            }

            groups_num += 1;

            if groups_num >= max_groups {
                first_group_name = Some(group_name.clone());
                break
            }
        }

        if let Some(first_group_name) = first_group_name {
            target_groups = target_groups.split_off(&first_group_name)
        }
    }

    target_groups
}