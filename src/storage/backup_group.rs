use std::collections::HashSet;

use regex::{self, Regex};

use crate::core::GenericResult;
use crate::provider::{ReadProvider, FileType};

use super::backup::Backup;
use super::helpers::BackupFileTraits;

pub struct BackupGroup {
    pub name: String,
    pub backups: Vec<Backup>,
}

impl BackupGroup {
    pub fn list(provider: &dyn ReadProvider, path: &str) -> GenericResult<(Vec<BackupGroup>, bool)> {
        let mut ok = true;
        let mut backup_groups = Vec::new();
        let name_regex = Regex::new(r"^\d{4}\.\d{2}\.\d{2}$")?;

        let mut files = provider.list_directory(path)?.ok_or_else(|| format!(
            "{:?} backup root doesn't exist", path))?;
        files.sort_by(|a, b| a.name.cmp(&b.name));

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            if file.type_ != FileType::Directory || !name_regex.is_match(&file.name) {
                error!("{:?} backup root on {} contains an unexpected {}: {:?}.",
                       path, provider.name(), file.type_, file.name);
                ok = false;
                continue;
            }

            let group_name = &file.name;
            let group_path = format!("{}/{}", path, group_name);

            let (group, group_ok) = BackupGroup::read(provider, group_name, &group_path).map_err(|e| format!(
                "Unable to list {:?} backup group: {}", group_path, e))?;
            ok &= group_ok;

            backup_groups.push(group);
        }

        Ok((backup_groups, ok))
    }

    fn read(provider: &dyn ReadProvider, name: &str, path: &str) -> GenericResult<(BackupGroup, bool)> {
        let mut ok = true;
        let mut first = true;

        let mut group = BackupGroup {
            name: name.to_owned(),
            backups: Vec::new(),
        };
        let backup_file_traits = BackupFileTraits::get_for(provider.type_());

        let mut files = provider.list_directory(path)?.ok_or_else(||
            "The backup group doesn't exist".to_owned())?;
        files.sort_by(|a, b| a.name.cmp(&b.name));

        for file in files {
            if file.name.starts_with('.') {
                continue
            }

            let captures = backup_file_traits.name_re.captures(&file.name);
            if file.type_ != backup_file_traits.type_ || captures.is_none() {
                error!("{:?} backup group on {} contains an unexpected {}: {:?}.",
                       path, provider.name(), file.type_, file.name);
                ok = false;
                continue
            }

            let backup_name = captures.unwrap().get(1).unwrap().as_str();
            let backup_path = format!("{}/{}", path, file.name);

            if first {
                first = false;

                if backup_name.split('-').next().unwrap() != group.name {
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
                    error!("{:?} backup on {} reading error: {}.", backup_path, provider.name(), e);
                    ok = false;
                    continue
                }
            };

            group.backups.push(backup);
        }

        Ok((group, ok))
    }

    pub fn inspect(&mut self, provider: &dyn ReadProvider) -> bool {
        let mut ok = true;
        let mut available_checksums = HashSet::new();

        for backup in &mut self.backups {
            match backup.inspect(provider, &mut available_checksums) {
                Ok(recoverable) => ok = ok && recoverable,
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