use std::path::PathBuf;

use clap::{App, Arg, ArgMatches, AppSettings, SubCommand};

use crate::core::GenericResult;

use super::Action;

pub struct Parser<'a> {
    matches: ArgMatches<'a>,
}

pub struct GlobalOptions {
    pub log_level: log::Level,
    pub config_path: String,
}

impl<'a> Parser<'a> {
    pub fn new() -> Parser<'a> {
        // Box is used to guarantee that Parser's memory won't be moved to preserve ArgMatches
        // lifetime requirements.
        Parser {
            matches: ArgMatches::new(),
        }
    }

    pub fn parse_global(&mut self) -> GenericResult<GlobalOptions> {
        let default_config_path = "~/.vsb.yaml";
        self.matches = App::new("Very Simple Backup")
            .about("\nVery simple in configuring but powerful backup tool")

            .arg(Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("PATH")
                .takes_value(true)
                .help(&format!("Configuration file path [default: {}]", default_config_path)))

            .arg(Arg::with_name("verbose")
                .short("v")
                .long("verbose")
                .multiple(true)
                .help("Sets verbosity level"))

            .subcommand(SubCommand::with_name("backup")
                .about("Run backup process for the specified backup name")
                .arg(Arg::with_name("NAME")
                    .help("Backup name")
                    .required(true)))

            .subcommand(SubCommand::with_name("restore")
                .about("Restore the specified backup")
                .arg(Arg::with_name("BACKUP_PATH")
                    .help("Backup path")
                    .required(true))
                .arg(Arg::with_name("RESTORE_PATH")
                    .help("Path to restore the backup to")
                    .required(true)))

            .subcommand(SubCommand::with_name("upload")
                .about("Upload backups to cloud"))

            .global_setting(AppSettings::DisableVersion)
            .global_setting(AppSettings::DisableHelpSubcommand)
            .global_setting(AppSettings::DeriveDisplayOrder)
            .setting(AppSettings::SubcommandRequiredElseHelp)
            .get_matches();

        let log_level = match self.matches.occurrences_of("verbose") {
            0 => log::Level::Info,
            1 => log::Level::Debug,
            2 => log::Level::Trace,
            _ => return Err!("Invalid verbosity level"),
        };

        let config_path = self.matches.value_of("config").map(ToString::to_string).unwrap_or_else(||
            shellexpand::tilde(default_config_path).to_string());

        Ok(GlobalOptions {log_level, config_path})
    }

    pub fn parse(self) -> GenericResult<Action> {
        let (command, matches) = self.matches.subcommand();
        let matches = matches.unwrap();

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