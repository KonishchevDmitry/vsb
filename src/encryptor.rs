use std::fs::File;
use std::io::{self, Read, BufReader, BufRead, Write, BufWriter};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::process::{Command, Stdio, Child, ChildStdin, ChildStdout};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time;

use libc::pid_t;
use nix::{fcntl, unistd};

use core::{EmptyResult, GenericResult};
use hash::Hasher;
use stream_splitter::{DataSender, DataReceiver, Data};
use util;

pub struct Encryptor {
    pid: pid_t,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout_reader: Option<JoinHandle<GenericResult<String>>>,
    encrypted_data_tx: Option<DataSender>,
    result: Option<EmptyResult>,
}

impl Encryptor {
    pub fn new(encryption_passphrase: &str, hasher: Box<dyn Hasher>) -> GenericResult<(Encryptor, DataReceiver)> {
        // Buffer is for the following reasons:
        // 1. Parallelization.
        // 2. To not block in drop() if we get some error during dropping the object that hasn't
        //    been used yet (hasn't been written to):
        //    * One buffer slot for gpg overhead around an empty payload.
        //    * One buffer slot for our error message.
        let (tx, rx) = mpsc::sync_channel(2);

        let (passphrase_read_fd, mut passphrase_write_fd) = create_passphrase_pipe()
            .map_err(|e| format!("Unable to create a pipe: {}", e))?;

        debug!("Spawning a gpg process to handle data encryption...");

        let mut gpg = Command::new("gpg")
            .arg("--batch").arg("--symmetric")
            .arg("--passphrase-fd").arg(passphrase_read_fd.as_raw_fd().to_string())
            .arg("--compress-algo").arg("none")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|e| format!("Unable to spawn a gpg process: {}", e))?;
        drop(passphrase_read_fd);

        let pid = gpg.id() as pid_t;
        let stdin = BufWriter::new(gpg.stdin.take().unwrap());
        let encrypted_chunks_tx = tx.clone();

        let stdout_reader = util::spawn_thread("gpg stdout reader", move || {
            stdout_reader(gpg, hasher, tx)
        }).map_err(|e| {
            terminate_gpg(pid);
            e
        })?;

        let encryptor = Encryptor {
            pid: pid,
            stdin: Some(stdin),
            stdout_reader: Some(stdout_reader),
            encrypted_data_tx: Some(encrypted_chunks_tx),
            result: None,
        };

        if let Err(err) = passphrase_write_fd.write_all(encryption_passphrase.as_bytes())
            .and_then(|_| passphrase_write_fd.flush()) {
            drop(passphrase_write_fd);
            encryptor.finish(None)?; // Try to get the real error here
            return Err!("Failed to pass encryption passphrase to gpg: {}", err);
        }

        Ok((encryptor, rx))
    }

    pub fn finish(mut self, error: Option<String>) -> EmptyResult {
        self.close(error.map_or(Ok(()), |e| Err(e.into())))
    }

    fn close(&mut self, mut result: EmptyResult) -> EmptyResult {
        if let Some(ref result) = self.result {
            return clone_empty_result(result);
        }

        debug!("Closing encryptor with {:?}...", result);

        if let Some(mut stdin) = self.stdin.take() {
            if let Err(err) = stdin.flush() {
                result = Err(err.into());
            }

            // Here stdin will be dropped and thus closed, so the gpg process will be expected to
            // read the remaining data and finish its work as well as our stdout reading thread.
        }

        if let Some(stdout_reader) = self.stdout_reader.take() {
            let tx = self.encrypted_data_tx.take().unwrap();

            let message = match util::join_thread(stdout_reader) {
                Ok(checksum) => {
                    match result {
                        Ok(_) => Ok(Data::EofWithChecksum(checksum)),
                        Err(ref err) => Err(err.to_string()),
                    }
                },
                Err(err) => {
                    result = Err(err.to_string().into());
                    terminate_gpg(self.pid);
                    Err(err.to_string())
                },
            };

            let _ = tx.send(message);
        }

        debug!("Encryptor has been closed with {:?}.", result);
        self.result = Some(clone_empty_result(&result));

        result
    }
}

impl Drop for Encryptor {
    fn drop(&mut self) {
        let _ = self.close(Err!("The encryptor has been dropped without finalization"));
    }
}

impl io::Write for Encryptor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(ref result) = self.result {
            return Err(io_error_from(result.as_ref().unwrap_err()));
        }

        self.stdin.as_mut().unwrap().write(buf).map_err(|e| {
            io_error_from(self.close(Err(e.into())).unwrap_err())
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(ref result) = self.result {
            return Err(io_error_from(result.as_ref().unwrap_err()));
        }

        self.stdin.as_mut().unwrap().flush().map_err(|e| {
            io_error_from(self.close(Err(e.into())).unwrap_err())
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn create_passphrase_pipe() -> nix::Result<(File, File)> {
    let (read_fd, write_fd) = unistd::pipe2(fcntl::OFlag::O_CLOEXEC).map(|(read_fd, write_fd)| {
        unsafe {
            (File::from_raw_fd(read_fd), File::from_raw_fd(write_fd))
        }
    })?;

    fcntl::fcntl(read_fd.as_raw_fd(), fcntl::FcntlArg::F_SETFD(fcntl::FdFlag::empty()))?;

    Ok((read_fd, write_fd))
}

#[cfg(target_os = "macos")]
fn create_passphrase_pipe() -> nix::Result<(File, File)> {
    let (read_fd, write_fd) = unistd::pipe().map(|(read_fd, write_fd)| {
        unsafe {
            (File::from_raw_fd(read_fd), File::from_raw_fd(write_fd))
        }
    })?;

    fcntl::fcntl(write_fd.as_raw_fd(), fcntl::FcntlArg::F_SETFD(fcntl::FdFlag::FD_CLOEXEC))?;

    Ok((read_fd, write_fd))
}

fn stdout_reader(mut gpg: Child, hasher: Box<dyn Hasher>, tx: DataSender) -> GenericResult<String> {
    let stdout = BufReader::new(gpg.stdout.take().unwrap());
    let mut stderr = gpg.stderr.take().unwrap();

    let mut stderr_reader = Some(util::spawn_thread("gpg stderr reader", move || -> EmptyResult {
        let mut error = String::new();

        match stderr.read_to_string(&mut error) {
            Ok(size) => {
                if size == 0 {
                    Ok(())
                } else {
                    Err!("gpg error: {}", error.trim_end())
                }
            },
            Err(err) => Err!("gpg stderr reading error: {}", err),
        }
    })?);

    let checksum = read_data(stdout, hasher, tx).map_err(|err| {
        terminate_gpg(gpg.id() as i32); // To close gpg's stderr
        util::join_thread_ignoring_result(stderr_reader.take().unwrap());
        err
    })?;

    util::join_thread(stderr_reader.take().unwrap())?;

    let status = gpg.wait().map_err(|e| format!("Failed to wait() a child gpg process: {}", e))?;
    if !status.success() {
        return Err!("gpg process has terminated with an error exit code");
    }

    debug!("gpg process has end its work with successful exit code.");

    Ok(checksum)
}

fn read_data(mut stdout: BufReader<ChildStdout>, mut hasher: Box<dyn Hasher>, tx: DataSender) -> GenericResult<String> {
    loop {
        let size = {
            let encrypted_data = stdout.fill_buf().map_err(|e| format!(
                "gpg stdout reading error: {}", e))?;

            if encrypted_data.is_empty() {
                return Ok(hasher.finish());
            }

            hasher.write_all(encrypted_data).map_err(|e| format!(
                "Unable to hash encrypted data: {}", e))?;

            tx.send(Ok(Data::Payload(encrypted_data.into()))).map_err(|_|
                "Unable to send encrypted data: the receiver has been closed".to_owned())?;

            encrypted_data.len()
        };

        stdout.consume(size);
    }
}

fn terminate_gpg(pid: pid_t) {
    let termination_timeout = time::Duration::from_secs(3);
    if let Err(err) = util::terminate_process("a child gpg process", pid, termination_timeout) {
        error!("{}.", err)
    }
}

fn io_error_from<T: ToString>(error: T) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error.to_string())
}

fn clone_empty_result(result: &EmptyResult) -> EmptyResult {
    match *result {
        Ok(()) => Ok(()),
        Err(ref err) => Err(err.to_string().into()),
    }
}