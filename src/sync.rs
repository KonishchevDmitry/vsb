use log;
use tar;

use core::EmptyResult;
use encryptor::Encryptor;
use storage::{Storage, BackupGroups, Backups};

// FIXME: + check logic
pub fn sync_backups(local_storage: &Storage, cloud_storage: &mut Storage) -> EmptyResult {
    let local_backup_groups = local_storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", local_storage.provider_name(), e))?;

    let cloud_backup_groups = cloud_storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", cloud_storage.provider_name(), e))?;

    if log_enabled!(log::LogLevel::Debug) {
        for &(storage, backup_groups) in &[
            (local_storage, &local_backup_groups),
            (cloud_storage, &cloud_backup_groups)
        ] {
            debug!("Backup groups on {}:", storage.provider_name());
            for (group_name, backups) in backup_groups.iter() {
                debug!("{}: {}", group_name, backups.iter().cloned().collect::<Vec<String>>().join(", "));
            }
        }
    }

    // FIXME: max
    let target_backup_groups = get_target_backup_groups(local_backup_groups, &cloud_backup_groups, 1);

    let no_backups = Backups::new();

    for (group_name, target_backup_names) in target_backup_groups.iter() {
        let cloud_backup_names = match cloud_backup_groups.get(group_name) {
            Some(backups) => backups,
            None => {
                info!("Creating {} backup group on {}...", group_name, cloud_storage.provider_name());
                &no_backups
            },
        };

        for backup_name in target_backup_names.iter() {
            if !cloud_backup_names.contains(backup_name) {
                info!("Uploading {}/{} backup to {}...",
                    group_name, backup_name, cloud_storage.provider_name());
            }
        }
    }

    for (group_name, _) in cloud_backup_groups.iter() {
        if !target_backup_groups.contains_key(group_name) {
            info!("Deleting {} backup group from {}...", group_name, cloud_storage.provider_name());
        }
    }

    if false {
        let (encryptor, chunks) = Encryptor::new().unwrap();
        drop(chunks);

        let mut archive = tar::Builder::new(encryptor);
        archive.append_dir_all("backup", "backup-mock").unwrap();

        let encryptor = archive.into_inner().unwrap();
        encryptor.finish().map_err(|e| format!("Got an error: {}", e)).unwrap();
    }

    Ok(())
}

// FIXME: check logic
fn get_target_backup_groups(local_backup_groups: BackupGroups, cloud_backup_groups: &BackupGroups, max_backup_groups: usize) -> BackupGroups {
    let mut target_backup_groups = local_backup_groups;

    for (group_name, backups) in cloud_backup_groups.iter() {
        target_backup_groups.entry(group_name.clone()).or_insert_with(Backups::new).extend(
            backups.iter().cloned());
    }

    if target_backup_groups.len() > max_backup_groups {
        let mut groups_num = 0;
        let mut first_group_name = None;

        for (group_name, backups) in target_backup_groups.iter().rev() {
            if backups.len() == 0 {
                continue
            }

            groups_num += 1;

            if groups_num >= max_backup_groups {
                first_group_name = Some(group_name.clone());
                break
            }
        }

        if let Some(first_group_name) = first_group_name {
            target_backup_groups = target_backup_groups.split_off(&first_group_name)
        }
    }

    target_backup_groups
}