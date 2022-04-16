use std::collections::HashSet;

use log::{error, warn};

use crate::core::GenericResult;
use crate::providers::{ReadProvider, FileType};

use super::backup::Backup;
use super::traits::BackupTraits;

pub struct BackupGroup {
    pub name: String,
    pub backups: Vec<Backup>,
    pub temporary_backups: Vec<Backup>,
}

impl BackupGroup {
    pub fn new(name: &str) -> BackupGroup {
        BackupGroup {
            name: name.to_owned(),
            backups: Vec::new(),
            temporary_backups: Vec::new(),
        }
    }

    pub fn list(provider: &dyn ReadProvider, path: &str) -> GenericResult<(Vec<BackupGroup>, bool)> {
        let traits = BackupTraits::get_for(provider.type_());

        let mut ok = true;
        let mut backup_groups = Vec::new();

        let mut files = provider.list_directory(path)?.ok_or_else(|| format!(
            "{:?} backup root doesn't exist", path))?;
        files.sort_by(|a, b| a.name.cmp(&b.name));

        for file in files {
            if file.name.starts_with('.') {
                // Assume OS-dependent hidden file
                continue
            }

            if file.type_ != FileType::Directory || !traits.group_name_regex.is_match(&file.name) {
                error!("{:?} backup root{} contains an unexpected {}: {:?}.",
                       path, provider.clarification(), file.type_, file.name);
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
        let traits = BackupTraits::get_for(provider.type_());

        let mut files = provider.list_directory(path)?.ok_or_else(||
            "The backup group doesn't exist".to_owned())?;
        files.sort_by(|a, b| a.name.cmp(&b.name));

        for file in files {
            let (stripped_file_name, temporary) = match file.name.strip_prefix(traits.temporary_prefix) {
                Some(stripped_name) => (stripped_name, true),
                None => (file.name.as_str(), false),
            };

            let backup_name = match traits.name_regex.captures(stripped_file_name) {
                Some(captures) if file.type_ == traits.file_type => {
                    // captures.name("name").unwrap().as_str()
                    captures.get(0).unwrap().as_str()
                },
                None if file.name.starts_with('.') => {
                    // Assume OS-dependent hidden file
                    continue
                },
                _ => {
                    error!("{:?} backup group{} contains an unexpected {}: {:?}.",
                        path, provider.clarification(), file.type_, file.name);
                    ok = false;
                    continue
                },
            };

            let backup_path = format!("{}/{}", path, file.name);
            if temporary {
                warn!("{:?} backup group{} contains a temporary {:?} backup.",
                    path, provider.clarification(), backup_name);
                group.temporary_backups.push(Backup::new(&backup_path, backup_name));
                continue
            }

            if first_backup {
                first_backup = false;

                if cfg!(not(test)) && backup_name.split('-').next().unwrap() != group.name {
                    error!(concat!(
                        "Suspicious first backup {:?} in {:?} group{}: ",
                        "possibly corrupted backup group."
                    ), backup_name, group.name, provider.clarification());
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
                    error!("{:?} backup{} reading error: {}.", backup_path, provider.clarification(), e);
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
                    error!("{:?} backup{} validation error: {}.",
                           backup.path, provider.clarification(), err);
                    ok = false;
                }
            };
        }

        ok
    }
}