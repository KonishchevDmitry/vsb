use std::path::PathBuf;

use clap::{Command, Arg, ArgAction, ArgMatches, value_parser};
use const_format::formatcp;

use crate::core::GenericResult;

use super::Action;

pub struct Parser {
    matches: Option<ArgMatches>,
}

pub struct GlobalOptions {
    pub log_level: log::Level,
    pub config_path: PathBuf,
}

impl Parser {
    pub fn new() -> Parser {
        Parser {matches: None}
    }

    pub fn parse_global(&mut self) -> GenericResult<GlobalOptions> {
        const DEFAULT_CONFIG_PATH: &str = "~/.vsb.yaml";

        let matches = Command::new("vsb")
            .about("Very Simple Backup")
            .version(env!("CARGO_PKG_VERSION"))

            .subcommand_required(true)
            .arg_required_else_help(true)
            .disable_help_subcommand(true)

            .dont_collapse_args_in_usage(true)
            .help_expected(true)

            .arg(Arg::new("config").short('c').long("config")
                .value_name("PATH")
                .value_parser(value_parser!(PathBuf))
                .help(formatcp!("Configuration file path [default: {}]", DEFAULT_CONFIG_PATH)))

            .arg(Arg::new("cron").long("cron")
                .action(ArgAction::SetTrue)
                .help("Show only warning and error messages (intended to be used from cron)"))

            .arg(Arg::new("verbose")
                .short('v').long("verbose")
                .conflicts_with("cron")
                .action(ArgAction::Count)
                .help("Set verbosity level"))

            .subcommand(Command::new("backup")
                .about("Run backup process for the specified backup name")
                .arg(Arg::new("NAME")
                    .help("Backup name")
                    .required(true)))

            .subcommand(Command::new("restore")
                .about("Restore the specified backup")
                .arg(Arg::new("BACKUP_PATH")
                    .value_parser(value_parser!(PathBuf))
                    .help("Backup path")
                    .required(true))
                .arg(Arg::new("RESTORE_PATH")
                    .value_parser(value_parser!(PathBuf))
                    .help("Path to restore the backup to")
                    .required(true)))

            .subcommand(Command::new("upload")
                .about("Upload backups to cloud")
                .arg(Arg::new("skip_verify").long("skip-verify")
                    .action(ArgAction::SetTrue)
                    .help("Skip backup verification before uploading")))

            .get_matches();

        let log_level = match matches.get_count("verbose") {
            0 => if matches.get_flag("cron") {
                log::Level::Warn
            } else {
                log::Level::Info
            },
            1 => log::Level::Debug,
            2 => log::Level::Trace,
            _ => return Err!("Invalid verbosity level"),
        };

        let config_path = matches.get_one("config").cloned().unwrap_or_else(||
            PathBuf::from(shellexpand::tilde(DEFAULT_CONFIG_PATH).to_string()));

        self.matches.replace(matches);

        Ok(GlobalOptions {log_level, config_path})
    }

    pub fn parse(self) -> GenericResult<Action> {
        let (command, matches) = self.matches.as_ref().unwrap().subcommand().unwrap();

        Ok(match command {
            "backup" => Action::Backup {
                name: matches.get_one("NAME").cloned().unwrap(),
            },

            "restore" => Action::Restore {
                backup_path: matches.get_one("BACKUP_PATH").cloned().unwrap(),
                restore_path: matches.get_one("RESTORE_PATH").cloned().unwrap(),
            },

            "upload" => Action::Upload {
                verify: !matches.get_flag("skip_verify"),
            },

            _ => unreachable!(),
        })
    }
}