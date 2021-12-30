// FIXME(konishchev): Drop
#![feature(io_error_more)]

// FIXME(konishchev): Refactor modules
#[macro_use] mod core;
mod backuping;
mod check;
mod cli;
mod config;
mod encryptor;
mod hash;
mod http_client;
mod metadata;
mod metrics;
mod oauth;
mod provider;
mod providers;
mod restoring;
mod storage;
mod stream_splitter;
mod sync;
mod uploader;
mod util;

use std::io::{self, Write};
use std::process;

use log::error;

use crate::cli::{Parser, GlobalOptions, Action};
use crate::config::Config;
use crate::core::GenericResult;

fn main() {
    let mut parser = Parser::new();

    let global = parser.parse_global().unwrap_or_else(|e| {
        let _ = writeln!(io::stderr(), "{}.", e);
        process::exit(1);
    });

    if let Err(e) = easy_logging::init(module_path!().split("::").next().unwrap(), global.log_level) {
        let _ = writeln!(io::stderr(), "Failed to initialize the logging: {}.", e);
        process::exit(1);
    }

    let ok = run(global, parser).unwrap_or_else(|e| {
        error!("{}.", e);
        false
    });

    process::exit(match ok {
        true => 0,
        false => 1,
    });
}

fn run(global: GlobalOptions, parser: Parser) -> GenericResult<bool> {
    let config_path = &global.config_path;
    let config = Config::load(config_path).map_err(|e| format!(
        "Error while reading {:?} configuration file: {}", config_path, e))?;

    match parser.parse()? {
        Action::Backup {name} => backuping::backup(config.get_backup(&name)?),
        Action::Upload => uploader::upload(&config),
    }
}