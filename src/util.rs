use std::thread;
use std::time::{self, Duration};

use libc::pid_t;
use log::{debug, error};
use nix::errno::Errno;
use nix::{sys, unistd};

use crate::core::{EmptyResult, GenericResult};

pub fn spawn_thread<F, T>(name: &str, f: F) -> GenericResult<thread::JoinHandle<T>>
    where F: FnOnce() -> T, F: Send + 'static, T: Send + 'static
{
    thread::Builder::new().name(name.to_owned()).spawn(f).map_err(|e| format!(
        "Unable to spawn a thread: {}", e).into())
}

pub fn join_thread<T>(handle: thread::JoinHandle<GenericResult<T>>) -> GenericResult<T> {
    let name = get_thread_name(handle.thread());
    match handle.join() {
        Ok(result) => result,
        Err(err) => {
            let error = format!("{:?} thread has panicked: {:?}", name, err);
            error!("{}.", error);
            Err(error.into())
        },
    }
}

pub fn join_thread_ignoring_result<T>(handle: thread::JoinHandle<T>) {
    let name = get_thread_name(handle.thread());
    if let Err(err) = handle.join() {
        error!("{:?} thread has panicked: {:?}.", name, err)
    }
}

fn get_thread_name(thread: &thread::Thread) -> String {
    match thread.name() {
        Some(name) => name.to_owned(),
        None => format!("{:?}", thread.id()),
    }
}

pub fn terminate_process(name: &str, pid: pid_t, timeout: Duration) -> EmptyResult {
    debug!("Terminating {}...", name);

    let pid = unistd::Pid::from_raw(pid);
    let mut signal = sys::signal::SIGTERM;
    let start_time = time::Instant::now();

    loop {
        match sys::signal::kill(pid, signal) {
            Ok(_) => {
                if signal != sys::signal::SIGKILL && start_time.elapsed() >= timeout {
                    error!("Failed to terminate {} using SIGTERM. Using SIGKILL...", name);
                    signal = sys::signal::SIGKILL;
                }

                match sys::wait::waitpid(pid, Some(sys::wait::WaitPidFlag::WNOHANG)) {
                    Ok(_) => break,
                    Err(Errno::ECHILD) => (),
                    Err(err) => return Err!("Failed to wait() {}: {}", name, err),
                };

                thread::sleep(Duration::from_millis(100));
            },
            Err(Errno::ESRCH) => break,
            Err(err) => return Err!("Failed to terminate {}: {}", name, err),
        }
    }

    debug!("Successfully terminated {}.", name);

    Ok(())
}