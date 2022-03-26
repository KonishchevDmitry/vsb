use std::collections::HashSet;

use log::error;

use crate::core::GenericResult;
use crate::provider::{ReadProvider, FileType};

use super::backup::Backup;
use super::traits::BackupTraits;

pub struct BackupGroup {
    pub name: String,
    pub backups: Vec<Backup>,
}

impl BackupGroup {
    pub fn new(name: &str) -> BackupGroup {
        BackupGroup {
            name: name.to_owned(),
            backups: Vec::new(),
        }
    }

    pub fn list(provider: &dyn ReadProvider, path: &str) -> GenericResult<(Vec<BackupGroup>, bool)> {
        let backup_traits = BackupTraits::get_for(provider.type_());

        let mut ok = true;
        let mut backup_groups = Vec::new();

        let mut files = provider.list_directory(path)?.ok_or_else(|| format!(
            "{:?} backup root doesn't exist", path))?;
        files.sort_by(|a, b| a.name.cmp(&b.name));

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            if file.type_ != FileType::Directory || !backup_traits.group_name_regex.is_match(&file.name) {
                error!("{:?} backup root on {} contains an unexpected {}: {:?}.",
                       path, provider.name(), file.type_, file.name);
                ok = false;
                continue;
            }

            let group_name = &file.name;
            let group_path = format!("{}/{}", path, group_name);

            let (group, group_ok) = BackupGroup::read(provider, group_name, &group_path, false).map_err(|e| format!(
                "Unable to list {:?} backup group: {}", group_path, e))?;
            ok &= group_ok;

            backup_groups.push(group);
        }

        Ok((backup_groups, ok))
    }

    pub fn read(provider: &dyn ReadProvider, name: &str, path: &str, strict: bool) -> GenericResult<(BackupGroup, bool)> {
        let mut ok = true;
        let mut first_backup = true;

        let mut group = BackupGroup::new(name);
        let backup_traits = BackupTraits::get_for(provider.type_());

        let mut files = provider.list_directory(path)?.ok_or_else(||
            "The backup group doesn't exist".to_owned())?;
        files.sort_by(|a, b| a.name.cmp(&b.name));

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            let captures = backup_traits.name_regex.captures(&file.name);
            if file.type_ != backup_traits.file_type || captures.is_none() {
                error!("{:?} backup group on {} contains an unexpected {}: {:?}.",
                       path, provider.name(), file.type_, file.name);
                ok = false;
                continue
            }

            let backup_name = captures.unwrap().get(1).unwrap().as_str();
            let backup_path = format!("{}/{}", path, file.name);

            if first_backup {
                first_backup = false;

                if cfg!(not(test)) && backup_name.split('-').next().unwrap() != group.name {
                    error!(concat!(
                        "Suspicious first backup {:?} in {:?} group on {}: ",
                        "possibly corrupted backup group."
                    ), backup_name, group.name, provider.name());
                    ok = false;
                }
            }

            let backup = match Backup::read(
                provider, backup_name, &backup_path,
                file.type_ != FileType::Directory
            ) {
                Ok(backup) => backup,
                Err(e) => {
                    if strict {
                        return Err!("Error while reading {:?} backup: {}", backup_path, e);
                    }
                    error!("{:?} backup on {} reading error: {}.", backup_path, provider.name(), e);
                    ok = false;
                    continue;
                }
            };

            group.backups.push(backup);
        }

        Ok((group, ok))
    }

    pub fn inspect(&mut self, provider: &dyn ReadProvider) -> bool {
        let mut ok = true;
        let mut available_hashes = HashSet::new();

        for backup in &mut self.backups {
            match backup.inspect(provider, &mut available_hashes) {
                Ok(recoverable) => ok &= recoverable,
                Err(err) => {
                    error!("{:?} backup on {} validation error: {}.",
                           backup.path, provider.name(), err);
                    ok = false;
                }
            };
        }

        ok
    }
}