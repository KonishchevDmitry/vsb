use std::fmt;
use std::io::{self, Write};
use std::sync::Mutex;

use ansi_term::Color;
use atty;
use chrono;
use fern::{Dispatch, FormatCallback};
use log::{LogLevel, LogLevelFilter, SetLoggerError};

lazy_static! {
    static ref GLOBAL_CONTEXT: Mutex<Option<String>> = Mutex::new(None);
    static ref OUTPUT_MUTEX: Mutex<()> = Mutex::new(());
}

pub struct GlobalContext {
}

impl GlobalContext {
    pub fn new(name: &str) -> GlobalContext {
        let context_string = format!("[{}] ", name);

        {
            let mut context = GLOBAL_CONTEXT.lock().unwrap();
            if context.is_some() {
                panic!("An attempt to set a nested global context.");
            }
            *context = Some(context_string);
        }

        GlobalContext{}
    }

    fn get() -> String {
        GLOBAL_CONTEXT.lock().unwrap().as_ref().map(Clone::clone).unwrap_or_else(String::new)
    }
}

impl Drop for GlobalContext {
    fn drop(&mut self) {
        *GLOBAL_CONTEXT.lock().unwrap() = None;
    }
}

pub fn init(level: LogLevel) -> Result<(), SetLoggerError> {
    let debug_mode = level >= LogLevel::Debug;

    let stdout_dispatcher =
        configure_formatter(Dispatch::new(), debug_mode, atty::is(atty::Stream::Stdout))
        .filter(|metadata| {metadata.level() >= LogLevel::Info})
        .chain(io::stdout());

    let stderr_dispatcher =
        configure_formatter(Dispatch::new(), debug_mode, atty::is(atty::Stream::Stderr))
        .filter(|metadata| {metadata.level() < LogLevel::Info})
        .chain(io::stderr());

    Dispatch::new()
        .level(if debug_mode {
            LogLevelFilter::Warn
        } else {
            LogLevelFilter::Off
        })
        .level_for("pyvsb_to_cloud", level.to_log_level_filter())
        .chain(stdout_dispatcher)
        .chain(stderr_dispatcher)
        .apply()
}

fn configure_formatter(dispatcher: Dispatch, debug_mode: bool, colored_output: bool) -> Dispatch {
    if debug_mode {
        dispatcher.format(move |out, message, record| {
            let location = record.location();
            let line = location.line();
            let level = record.level();

            let mut file_width = 10;
            let mut line_width = 3;
            let mut line_extra_width = line / 1000;

            while line_extra_width > 0 && file_width > 0 {
                line_width += 1;
                file_width -= 1;
                line_extra_width /= 10;
            }

            let mut file = location.file();
            if file.starts_with("src/") {
                file = &file[4..];
            }
            if file.len() > file_width {
                file = &file[file.len() - file_width..]
            }

            let level_name = get_level_name(level);
            let time = chrono::Local::now().format("[%T%.3f]");

            if colored_output {
                let level_color = get_level_color(level);
                write_log(out, level, format_args!(
                    "{color_prefix}{time} [{file:>file_width$}:{line:0line_width$}] {level}: {context}{message}{color_suffix}",
                    color_prefix=level_color.prefix(), time=time, file=file, file_width=file_width,
                    line=line, line_width=line_width, level=level_name,
                    context=GlobalContext::get(), message=message, color_suffix=level_color.suffix()
                ));
            } else {
                write_log(out, level, format_args!(
                    "{time} [{file:>file_width$}:{line:0line_width$}] {level}: {context}{message}",
                    time=time, file=file, file_width=file_width, line=line, line_width=line_width,
                    level=level_name, context=GlobalContext::get(), message=message
                ));
            }
        })
    } else {
        dispatcher.format(move |out, message, record| {
            let level = record.level();
            let level_name = get_level_name(level);

            if colored_output {
                let level_color = get_level_color(level);
                write_log(out, level, format_args!(
                    "{color_prefix}{level}: {context}{message}{color_suffix}",
                    color_prefix=level_color.prefix(), level=level_name,
                    context=GlobalContext::get(), message=message, color_suffix=level_color.suffix()
                ));
            } else {
                write_log(out, level, format_args!("{level}: {context}{message}",
                    level=level_name, context=GlobalContext::get(), message=message));
            }
        })
    }
}

fn get_level_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Error => Color::Red,
        LogLevel::Warn  => Color::Yellow,
        LogLevel::Info  => Color::Green,
        LogLevel::Debug => Color::Cyan,
        LogLevel::Trace => Color::Purple,
    }
}

fn get_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "E",
        LogLevel::Warn  => "W",
        LogLevel::Info  => "I",
        LogLevel::Debug => "D",
        LogLevel::Trace => "T",
    }
}

fn write_log(out: FormatCallback, level: LogLevel, formatted_message: fmt::Arguments) {
    // Since we write into stdout and stderr we should guard any write with a mutex to not get the
    // output interleaved.
    let _lock = OUTPUT_MUTEX.lock();

    out.finish(formatted_message);

    let _ = if level >= LogLevel::Info {
        io::stdout().flush()
    } else {
        io::stderr().flush()
    };
}