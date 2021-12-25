mod parser;

pub enum Action {
    Backup {name: String},
    Upload,
}

pub use parser::{Parser, GlobalOptions};