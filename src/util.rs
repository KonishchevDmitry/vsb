use std::thread;
use std::time::{self, Duration};

use nix::errno;
use nix::sys;

use core::EmptyResult;

pub fn join_thread<T>(handle: thread::JoinHandle<T>) {
    let name = if let Some(name) = handle.thread().name() {
        name.to_owned()
    } else {
        // FIXME: handle.thread().id() is stable since Rust 1.19
        format!("{:?}", 0)
    };

    if let Err(err) = handle.join() {
        error!("{} thread has panicked: {:?}.", name, err)
    }
}

pub fn terminate_process(name: &str, pid: i32, timeout: Duration) -> EmptyResult {
    debug!("Terminating {}...", name);

    let mut signal = sys::signal::SIGTERM;
    let start_time = time::Instant::now();

    loop {
        match sys::signal::kill(pid, signal) {
            Ok(_) => {
                if signal != sys::signal::SIGKILL && start_time.elapsed() >= timeout {
                    error!("Failed to terminate {} using SIGTERM. Using SIGKILL...", name);
                    signal = sys::signal::SIGKILL;
                }

                thread::sleep(Duration::from_millis(100));
            },
            Err(err) => {
                if err.errno() == errno::ESRCH {
                    break;
                } else {
                    return Err!("Failed to terminate {}: {}", name, err);
                }
            },
        }
    }

    debug!("Successfully terminated {}.", name);

    Ok(())
}