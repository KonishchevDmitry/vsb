mod parser;

use std::path::PathBuf;

pub use parser::{Parser, GlobalOptions};

pub enum Action {
    Backup {name: String},

    Restore {
        backup_path: PathBuf,
        restore_path: PathBuf,
    },

    Upload,
}