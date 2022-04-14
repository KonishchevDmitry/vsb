use std::path::PathBuf;

use clap::{AppSettings, Command, Arg, ArgMatches};
use const_format::formatcp;
use indoc::indoc;

use crate::core::GenericResult;

use super::Action;

pub struct Parser {
    matches: Option<ArgMatches>,
}

pub struct GlobalOptions {
    pub log_level: log::Level,
    pub config_path: String,
}

impl Parser {
    pub fn new() -> Parser {
        Parser {matches: None}
    }

    pub fn parse_global(&mut self) -> GenericResult<GlobalOptions> {
        const DEFAULT_CONFIG_PATH: &str = "~/.vsb.yaml";

        let matches = new_command("vsb", "Very Simple Backup")
            .version(env!("CARGO_PKG_VERSION"))

            .subcommand_required(true)
            .arg_required_else_help(true)
            .disable_help_subcommand(true)

            .global_setting(AppSettings::DeriveDisplayOrder)
            .dont_collapse_args_in_usage(true)
            .help_expected(true)

            .arg(Arg::new("config")
                .short('c')
                .long("config")
                .value_name("PATH")
                .takes_value(true)
                .help(formatcp!("Configuration file path [default: {}]", DEFAULT_CONFIG_PATH)))

            .arg(Arg::new("cron")
                .long("cron")
                .help("Show only warning and error messages (intended to be used from cron)"))

            .arg(Arg::new("verbose")
                .short('v').long("verbose")
                .conflicts_with("cron")
                .multiple_occurrences(true)
                .max_occurrences(2)
                .help("Set verbosity level"))

            .subcommand(new_command(
                "backup", "Run backup process for the specified backup name")
                .arg(Arg::new("NAME")
                    .help("Backup name")
                    .required(true)))

            .subcommand(new_command(
                "restore", "Restore the specified backup")
                .arg(Arg::new("BACKUP_PATH")
                    .help("Backup path")
                    .required(true))
                .arg(Arg::new("RESTORE_PATH")
                    .help("Path to restore the backup to")
                    .required(true)))

            .subcommand(new_command("upload", "Upload backups to cloud"))
            .get_matches();

        let log_level = match matches.occurrences_of("verbose") {
            0 => if matches.is_present("cron") {
                log::Level::Warn
            } else {
                log::Level::Info
            },
            1 => log::Level::Debug,
            2 => log::Level::Trace,
            _ => return Err!("Invalid verbosity level"),
        };

        let config_path = matches.value_of("config").map(ToString::to_string).unwrap_or_else(||
            shellexpand::tilde(DEFAULT_CONFIG_PATH).to_string());

        self.matches.replace(matches);

        Ok(GlobalOptions {log_level, config_path})
    }

    pub fn parse(self) -> GenericResult<Action> {
        let (command, matches) = self.matches.as_ref().unwrap().subcommand().unwrap();

        Ok(match command {
            "backup" => Action::Backup {
                name: matches.value_of("NAME").unwrap().to_owned(),
            },

            "restore" => Action::Restore {
                backup_path: PathBuf::from(matches.value_of("BACKUP_PATH").unwrap()),
                restore_path: PathBuf::from(matches.value_of("RESTORE_PATH").unwrap()),
            },

            "upload" => Action::Upload,

            _ => unreachable!(),
        })
    }
}

fn new_command<'help>(name: &str, about: &'help str) -> Command<'help> {
    Command::new(name)
        // Default template contains `{bin} {version}` for some reason
        .help_template(indoc!("
            {before-help}{about}

            {usage-heading}
                {usage}

            {all-args}{after-help}\
        "))
        .about(about)
}