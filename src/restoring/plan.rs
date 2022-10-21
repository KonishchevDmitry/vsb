use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use log::{error, info};

use crate::core::{GenericError, GenericResult};
use crate::storage::{Storage, Backup};
use crate::util::hash::Hash;

pub struct RestorePlan {
    pub steps: Vec<RestoreStep>,
    pub extern_files: HashSet<PathBuf>,
    pub missing_files: HashSet<PathBuf>,
}

pub struct RestoreStep {
    pub backup: Backup,
    pub files: HashMap<PathBuf, RestoringFile>,
}

pub struct RestoringFile {
    pub hash: Hash,
    pub size: u64,
    pub paths: Vec<PathBuf>,
}

impl RestorePlan {
    pub fn new(storage: &Storage, group_name: &str, backup_name: &str) -> GenericResult<(RestorePlan, bool)> {
        let mut ok = true;

        let provider = storage.provider.read();
        let group = storage.get_backup_group(group_name, true)?;

        let mut steps = Vec::new();
        let mut extern_files: HashSet<PathBuf> = HashSet::new();
        let mut to_find: HashMap<Hash, Vec<PathBuf>> = HashMap::new();

        info!("Building restoring plan...");

        for backup in group.backups.into_iter().rev() {
            if steps.is_empty() && backup.name != backup_name {
                continue;
            }

            let map_read_error = |error: GenericError| -> String {
                format!("Error while reading {:?} backup metadata: {}", backup.path, error)
            };

            let mut to_restore = HashMap::new();

            if steps.is_empty() {
                let mut own_files = Vec::new();

                for file in backup.read_metadata(provider).map_err(map_read_error)? {
                    let file = file.map_err(map_read_error)?;
                    let path = PathBuf::from(file.path);

                    if file.unique || file.size == 0 {
                        own_files.push((path, file.hash, file.size));
                    } else {
                        to_find.entry(file.hash).or_default().push(path);
                    }
                }

                for (path, hash, size) in own_files {
                    let mut paths = to_find.remove(&hash).unwrap_or_default();
                    extern_files.extend(paths.iter().cloned());

                    paths.reserve_exact(1);
                    paths.push(path.clone());
                    to_restore.insert(path, RestoringFile {hash, size, paths});
                }
            } else {
                if to_find.is_empty() {
                    break;
                }

                for file in backup.read_metadata(provider).map_err(map_read_error)? {
                    let file = file.map_err(map_read_error)?;
                    if !file.unique {
                        continue;
                    }

                    if let Some(paths) = to_find.remove(&file.hash) {
                        extern_files.extend(paths.iter().cloned());
                        to_restore.insert(file.path.into(), RestoringFile {
                            hash: file.hash,
                            size: file.size,
                            paths
                        });

                        if to_find.is_empty() {
                            break;
                        }
                    }
                }
            }

            if steps.is_empty() || !to_restore.is_empty() {
                steps.push(RestoreStep {backup, files: to_restore});
            }
        }

        if steps.is_empty() {
            return Err!("The backup doesn't exist");
        }

        let mut missing_files = HashSet::new();
        for paths in to_find.into_values() {
            missing_files.extend(paths);
        }

        if !missing_files.is_empty() {
            error!("The following files aren't recoverable (missing extern data):");
            for path in &missing_files {
                error!("* {}", path.to_string_lossy());
            }
            ok = false;
        }

        Ok((RestorePlan {steps, extern_files, missing_files}, ok))
    }
}