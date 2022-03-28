mod file_metadata;
mod multi_writer;
mod plan;
mod restorer;
mod util;

use std::path::Path;

use crate::core::GenericResult;

pub use restorer::Restorer;

// FIXME(konishchev): Implement
pub fn restore(backup_path: &Path, restore_dir: &Path) -> GenericResult<bool> {
    Restorer::new(backup_path)?.restore(restore_dir)
}