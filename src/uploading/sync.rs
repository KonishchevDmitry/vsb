use std::collections::{BTreeMap, BTreeSet};

use log::{info, warn, error};

use crate::core::EmptyResult;
use crate::storage::{Storage, BackupGroup};

pub fn sync_backups(
    local_storage: &Storage, local_groups: &[BackupGroup],
    cloud_storage: &Storage, cloud_groups: &[BackupGroup],
    mut ok: bool, max_backup_groups: usize, encryption_passphrase: &str,
) -> bool {
    if let Err(err) = check_backup_groups(local_groups, cloud_groups) {
        error!("{}.", err);
        ok = false;
    }

    let target_groups = get_target_backup_groups(local_groups, cloud_groups, max_backup_groups);
    let cloud_groups = get_group_to_backups_mapping(cloud_groups);
    let no_backups = BTreeSet::new();

    for (&group_name, target_backups) in target_groups.iter() {
        if target_backups.is_empty() {
            continue;
        }

        let cloud_backups = match cloud_groups.get(group_name) {
            Some(backups) => backups,
            None => {
                if let Err(err) = cloud_storage.create_backup_group(group_name) {
                    error!("Failed to create {:?} backup group on {}: {}.",
                           group_name, cloud_storage.name(), err);
                    ok = false;
                    continue;
                }

                &no_backups
            },
        };

        for &backup_name in target_backups {
            if cloud_backups.contains(backup_name) {
                continue;
            }

            let backup_path = local_storage.get_backup_path(group_name, backup_name, false);
            info!("Uploading {:?} backup to {}...", backup_path, cloud_storage.name());

            if let Err(err) = cloud_storage.upload_backup(
                &backup_path, group_name, backup_name, encryption_passphrase
            ) {
                error!("Failed to upload {:?} backup to {}: {}.",
                       backup_path, cloud_storage.name(), err);
                ok = false;
            }
        }
    }

    for &group_name in cloud_groups.keys() {
        if target_groups.contains_key(group_name) {
            continue
        }

        if !ok {
            warn!("Skipping deletion of {:?} backup group from {} because of the errors above.",
                  group_name, cloud_storage.name());
            continue;
        }

        info!("Deleting {:?} backup group from {}...", group_name, cloud_storage.name());
        if let Err(err) = cloud_storage.delete_backup_group(group_name) {
            error!("Failed to delete {:?} backup backup group from {}: {}.",
                   group_name, cloud_storage.name(), err)
        }
    }

    ok
}

fn check_backup_groups(local_groups: &[BackupGroup], cloud_groups: &[BackupGroup]) -> EmptyResult {
    let local_groups_num = local_groups.iter().filter(|group| !group.backups.is_empty()).count();
    let cloud_groups_num = cloud_groups.len();

    if local_groups_num < 2 && cloud_groups_num > local_groups_num {
        return Err!(
            "A possible backup corruption: Cloud contains more backup groups than stored locally")
    }

    Ok(())
}

fn get_target_backup_groups<'a>(
    local_groups: &'a [BackupGroup], cloud_groups: &'a [BackupGroup], max_groups: usize,
) -> BTreeMap<&'a str, BTreeSet<&'a str>> {
    let mut target_groups = get_group_to_backups_mapping(local_groups);

    for group in cloud_groups {
        target_groups.entry(&group.name).or_default().extend(
            group.backups.iter().map(|backup| backup.name.as_str()));
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
                first_group_name.replace(group_name.to_owned());
                break
            }
        }

        if let Some(first_group_name) = first_group_name {
            target_groups = target_groups.split_off(first_group_name)
        }
    }

    target_groups
}

fn get_group_to_backups_mapping(groups: &[BackupGroup]) -> BTreeMap<&str, BTreeSet<&str>> {
    groups.iter().map(|group| {
        let backups = group.backups.iter().map(|backup| backup.name.as_str()).collect();
        (group.name.as_str(), backups)
    }).collect()
}