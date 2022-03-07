// FIXME(konishchev): Handle file truncation during backing up properly

use std::collections::HashMap;
use std::path::Path;

use itertools::Itertools;
use serde_json::Value;

use crate::core::{EmptyResult, GenericResult};
use crate::hash::Hash;
use crate::storage::{Storage, StorageRc, BackupGroup, Backup};

pub struct Restorer {
    storage: StorageRc,
}

impl Restorer {
    // FIXME(konishchev): Rewrite
    pub fn new(storage: StorageRc) -> GenericResult<Restorer> {
        Ok(Restorer {storage})
    }

    // FIXME(konishchev): Rewrite
    pub fn restore(&self, group_name: &str, backup_name: &str, path: &Path) -> EmptyResult {
        let group_path = self.storage.get_backup_group_path(group_name);

        // FIXME(konishchev): ok
        let (mut group, _) = BackupGroup::read(self.storage.provider.read(), group_name, &group_path)?;

        // FIXME(konishchev): Unwrap
        let (position, _) = group.backups.iter().find_position(|backup| backup.name == backup_name).unwrap();
        let tail_size = group.backups.len() - position - 1;

        let mut extern_files: HashMap<Hash, Vec<String>> = HashMap::new();
        let mut restore_plan = Vec::new();

        for (index, backup) in group.backups.into_iter().rev().dropping(tail_size).enumerate() {
            if index == 0 {
                for file in backup.read_metadata(self.storage.provider.read())? {
                    let file = file?;
                    if !file.unique {
                        extern_files.entry(file.hash).or_default().push(file.path);
                    }
                }

                restore_plan.push(RestoreStep {backup, files: HashMap::new()})
            } else {
                if extern_files.is_empty() {
                    break;
                }

                let mut files = HashMap::new();

                for file in backup.read_metadata(self.storage.provider.read())? {
                    let file = file?;
                    if let Some(paths) = extern_files.remove(&file.hash) {
                        files.insert(file.path, paths);
                    }
                }

                if !files.is_empty() {
                    restore_plan.push(RestoreStep {backup, files})
                }
            }
        }

        if !extern_files.is_empty() {
            // FIXME(konishchev): Support
            unimplemented!();
        }

        self.restore_impl(restore_plan, path)?;
        Ok(())
    }

    // FIXME(konishchev): Rewrite
    // FIXME(konishchev): Implement
    fn restore_impl(&self, plan: Vec<RestoreStep>, path: &Path) -> EmptyResult {
        assert!(!plan.is_empty());

        let step = plan.first().unwrap();
        step.backup.read_data(self.storage.provider.read())?.unpack(path)?;

        Ok(())
    }
}

struct RestoreStep {
    backup: Backup,
    files: HashMap<String, Vec<String>>,
}