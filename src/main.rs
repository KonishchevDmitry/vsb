extern crate ansi_term;
extern crate atty;
extern crate chrono;
extern crate fern;
#[macro_use]extern crate log;

mod logging;

fn main() {
    logging::init().expect("Failed to initialize the logging");
    info!("ok")
}
