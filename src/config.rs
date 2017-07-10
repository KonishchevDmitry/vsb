use clap::{App, Arg, AppSettings};
use shellexpand;

use core::GenericResult;

pub struct Config {
}

pub fn load() -> GenericResult<Config> {
    let default_config_path = "~/.pyvsb_to_cloud";

    let matches = App::new("PyVSB to cloud")
        .about("\nUploads PyVSB backups to cloud")
        .arg(Arg::with_name("config")
            .short("c")
            .long("config")
            .value_name("PATH")
            .help(&format!("Configuration file path [default: {}]", default_config_path))
            .takes_value(true))
        .setting(AppSettings::DisableVersion)
        .get_matches();

    let config_path = matches.value_of("config").map(ToString::to_string).unwrap_or_else(||
        shellexpand::tilde(default_config_path).to_string());

    Ok(Config{})
}