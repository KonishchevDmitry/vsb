use std::fs::File;
use std::io::{self, Write, BufReader};
use std::process::{Command, Child, Stdio};
use std::thread::{self, JoinHandle};
use std::time;

use nix::{errno, sys};

use core::GenericResult;
use util;

pub struct Encryptor {
    gpg: Child,
    stdout_reader: JoinHandle<()>,
    stderr_reader: JoinHandle<()>,
}

impl Encryptor {
    // FIXME
    pub fn new() -> GenericResult<Encryptor> {
        // FIXME
        let mut gpg = Command::new("cat")//.arg("a")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn()?;

        let stdout = gpg.stdout.take().unwrap();
        let stdout_reader = thread::Builder::new().name("encryptor-stdout-reader".into()).spawn(move || {
        });

        let stderr = gpg.stdout.take().unwrap();
        let stderr_reader = thread::Builder::new().name("encryptor-stderr-reader".into()).spawn(move || {
        });

        if stdout_reader.is_err() || stderr_reader.is_err() {
            gpg.kill();
            gpg.wait();

            let error = stdout_reader.as_ref().err().unwrap_or_else(||
                stderr_reader.as_ref().err().unwrap()).to_string();

            if let Ok(stdout_reader) = stdout_reader {
                stdout_reader.join();
            }

            if let Ok(stderr_reader) = stderr_reader {
                stderr_reader.join();
            }

            return Err(From::from(error));
        }

        Ok(Encryptor {
            gpg: gpg,
            stdout_reader: stdout_reader.unwrap(),
            stderr_reader: stderr_reader.unwrap(),
        })
        // FIXME
//        let file = File::create("backup-mock.tar").unwrap();
    }
}

impl Drop for Encryptor {
    fn drop(&mut self) {
        // FIXME
        let status = match self.gpg.try_wait() {
            Ok(status) => status,
            Err(err) => {
                error!("Failed to wait() a child gpg process: {}.", err);
                return;
            }
        };

//        if let None = status {
//            util::terminate_process("a child gpg process", time::Duration::from_secs());
//        }
    }
}

// FIXME
impl io::Write for Encryptor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}