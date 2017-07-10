use std::collections::{HashSet, HashMap, BTreeMap};

use tar;

use core::EmptyResult;
use encryptor::Encryptor;
use storage::{Storage, BackupGroup};

// FIXME: + check logic
pub fn sync_backups(local_storage: &Storage, cloud_storage: &mut Storage) -> EmptyResult {
    let local_backup_groups = local_storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", local_storage.provider_name(), e))?;

    let cloud_backup_groups = cloud_storage.get_backup_groups().map_err(|e| format!(
        "Failed to list backup groups on {}: {}", cloud_storage.provider_name(), e))?;

    info!("> {:?}", local_backup_groups);
    info!("> {:?}", cloud_backup_groups);

    let target_backup_groups = get_target_backup_groups(&[&local_backup_groups, &cloud_backup_groups], 1);
    info!("> {:?}", target_backup_groups);

    let mut cloud_backup_groups_map = BTreeMap::new();
    map_backup_groups(&mut cloud_backup_groups_map, &cloud_backup_groups);

    let no_backups = HashSet::new();

    for (group_name, target_backup_names) in target_backup_groups.iter() {
        let cloud_backup_names = match cloud_backup_groups_map.get(group_name) {
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

    for (group_name, _) in cloud_backup_groups_map.iter() {
        if !target_backup_groups.contains_key(group_name) {
            info!("Deleting {} backup group from {}...", group_name, cloud_storage.provider_name());
        }
    }

    if false {
        info!("> {:?}", local_storage.get_backup_groups().unwrap());
        let (encryptor, chunks) = Encryptor::new().unwrap();
        drop(chunks);

        let mut archive = tar::Builder::new(encryptor);
        archive.append_dir_all("backup", "backup-mock").unwrap();

        let mut encryptor = archive.into_inner().unwrap();
        encryptor.finish().map_err(|e| format!("Got an error: {}", e)).unwrap();
    }

    Ok(())
}

// FIXME: check logic
fn get_target_backup_groups(backup_groups_list: &[&Vec<BackupGroup>], max_backup_groups: usize) -> BTreeMap<String, HashSet<String>> {
    let mut target_backup_groups = BTreeMap::new();
    backup_groups_list.iter().map(|backup_groups|
        map_backup_groups(&mut target_backup_groups, &backup_groups)).last();

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

fn map_backup_groups(backup_groups_map: &mut BTreeMap<String, HashSet<String>>, backup_groups: &Vec<BackupGroup>) {
    for backup_group in backup_groups.iter() {
        if !backup_groups_map.contains_key(&backup_group.name) {
            backup_groups_map.insert(backup_group.name.clone(), HashSet::new());
        }

        backup_groups_map.get_mut(&backup_group.name).unwrap().extend(
            backup_group.backups.iter().cloned());
    }
}