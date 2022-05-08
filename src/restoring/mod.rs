mod file_metadata;
mod multi_writer;
mod plan;
mod restorer;
mod users;
mod util;

use std::path::Path;

use crate::core::GenericResult;

use restorer::Restorer;

pub fn restore(backup_path: &Path, restore_dir: &Path) -> GenericResult<bool> {
    Restorer::new(backup_path)?.restore(restore_dir)
}