use std::fs::File;
use std::io::{self, Read, Write, BufReader};
use std::process::{Command, Stdio, Child, ChildStdin, ChildStdout};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time;

use nix::{errno, sys};

use core::{EmptyResult, GenericResult};
use util;

pub struct Encryptor {
    pid: Option<i32>,
    stdin: ChildStdin,
    stdout_reader: Option<JoinHandle<()>>,
    process_termination_event: mpsc::Receiver<Option<String>>,
}

impl Encryptor {
    pub fn new() -> GenericResult<Encryptor> {
        // FIXME
        let mut gpg = Command::new("cat")//.arg("a")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn()?;

        let (tx, rx) = mpsc::channel();

        let mut encryptor = Encryptor {
            pid: Some(gpg.id() as i32),
            stdin: gpg.stdin.take().unwrap(),
            stdout_reader: None,
            process_termination_event: rx,
        };

        match thread::Builder::new().name("encryptor-stdout-reader".into()).spawn(move || {
            let _ = tx.send(stdout_reader(gpg).map_err(|e| e.to_string()).err());
        }) {
            Ok(stdout_reader) => encryptor.stdout_reader = Some(stdout_reader),
            Err(err) => return Err!("Unable to spawn a thread: {}", err),
        }

        Ok(encryptor)
    }
}

impl Drop for Encryptor {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            terminate_gpg_by_pid(pid);
            self.pid = None;
        }

        if let Some(stdout_reader) = self.stdout_reader.take() {
            if let Err(err) = stdout_reader.join() {
                error!("gpg stdout thread has panicked: {:?}.", err);
            }
        }
        // FIXME
    }
}

impl io::Write for Encryptor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdin.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        // FIXME: ensure properly terminated
        self.stdin.flush()
    }
}

// FIXME
fn stdout_reader(mut gpg: Child) -> EmptyResult {
    let mut stderr = gpg.stderr.take().unwrap();

    let mut stderr_reader = Some(thread::Builder::new().name("encryptor-stderr-reader".into()).spawn(move || -> EmptyResult {
        let mut error = String::new();

        match stderr.read_to_string(&mut error) {
            Ok(size) => {
                if size == 0 {
                    Ok(())
                } else {
                    Err(From::from(error.trim_right()))
                }
            },
            Err(err) => Err!("Failed to read from gpg's stderr: {}", err),
        }
    }).map_err(|err| {
        terminate_gpg(&mut gpg);
        format!("Unable to spawn a thread: {}", err)
    })?);

    // FIXME: Buf size
    let stdout = BufReader::new(gpg.stdout.take().unwrap());

    read_data(stdout).map_err(|e| {
        terminate_gpg(&mut gpg);
        stderr_reader.take().unwrap().join();
        format!("gpg stdout reading error: {}", e)
    })?;

    stderr_reader.take().unwrap().join().map_err(|e| {
        terminate_gpg(&mut gpg);
        format!("gpg stderr reading thread has panicked: {}", 3)
    })??;

    let status = gpg.wait().map_err(|e| format!("Failed to wait() a child gpg process: {}", e))?;
    if !status.success() {
        return Err!("gpg process has terminated with a non-successful status code")
    }

    Ok(())
}

// FIXME
fn read_data(stdout: BufReader<ChildStdout>) -> EmptyResult {
    // FIXME
    let _ = File::create("backup-mock.tar").unwrap();

    Ok(())
}

fn terminate_gpg(gpg: &mut Child) {
    let pid = gpg.id() as i32;
    match gpg.try_wait() {
        Ok(Some(status)) => (),
        Ok(None) => terminate_gpg_by_pid(pid),
        Err(err) => {
            error!("Failed to wait() a child gpg process: {}", err);
            terminate_gpg_by_pid(pid);
        }
    };
}

fn terminate_gpg_by_pid(pid: i32) {
    let termination_timeout = time::Duration::from_secs(3);
    if let Err(err) = util::terminate_process("a child gpg process", pid, termination_timeout) {
        error!("{}.", err)
    }
}