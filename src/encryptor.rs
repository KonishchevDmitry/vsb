use std::fs::File;
use std::io::{self, Read, BufReader, BufRead, Write, BufWriter};
use std::process::{Command, Stdio, Child, ChildStdin, ChildStdout};
use std::thread::{self, JoinHandle};
use std::time;

use futures::{Future, Sink};
use futures::sync::mpsc;
use hyper::{self, Chunk};

use core::{EmptyResult, GenericResult};
use util;

pub struct Encryptor {
    pid: i32,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout_reader: Option<JoinHandle<EmptyResult>>,
    encrypted_chunks_tx: Option<mpsc::Sender<ChunkResult>>,
}

type ChunkResult = Result<Chunk, hyper::Error>;

impl Encryptor {
    pub fn new() -> GenericResult<(Encryptor, mpsc::Receiver<ChunkResult>)> {
        debug!("Spawning a gpg process to handle data encryption...");

        // One buffer slot is for parallelization or to not block in drop() if we get some error
        // during dropping the object that hasn't been used yet (hasn't been written to).
        let (tx, rx) = mpsc::channel(1);

        // FIXME
        let mut gpg = Command::new("cat")//.arg("a")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn()?;

        let pid = gpg.id() as i32;
        let stdin = BufWriter::new(gpg.stdin.take().unwrap());
        let encrypted_chunks_tx = tx.clone();

        let stdout_reader = thread::Builder::new().name("gpg stdout reader".into()).spawn(move || {
            stdout_reader(gpg, tx)
        }).map_err(|e| {
            terminate_gpg(pid);
            format!("Unable to spawn a thread: {}", e)
        })?;

        Ok((Encryptor {
            pid: pid,
            stdin: Some(stdin),
            stdout_reader: Some(stdout_reader),
            encrypted_chunks_tx: Some(encrypted_chunks_tx),
        }, rx))
    }

    pub fn finish(mut self) -> EmptyResult {
        self.close()
    }

    fn close(&mut self) -> EmptyResult {
        let mut result: EmptyResult = Ok(());

        if let Some(mut stdin) = self.stdin.take() {
            if let Err(err) = stdin.flush() {
                result = Err(From::from(err));
            }

            // Here stdin will be dropped and thus closed, so the gpg process will be expected to
            // read the remaining data and finish its work as well as our stdout reading thread.
        }

        if let Some(stdout_reader) = self.stdout_reader.take() {
            if let Err(err) = util::join_thread(stdout_reader) {
                result = Err(From::from(err));
                terminate_gpg(self.pid);
            }
        }

        if let Some(encrypted_chunks_tx) = self.encrypted_chunks_tx.take() {
            if let Err(ref err) = result {
                let _ = encrypted_chunks_tx.send(Err(hyper::Error::Io(io::Error::new(
                    io::ErrorKind::Other, err.to_string()))));
            }
        }

        return result;
    }
}

impl Drop for Encryptor {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl io::Write for Encryptor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdin.as_mut().unwrap().write(buf).map(|size| {
            trace!("Accepted {} bytes of data for encryption.", size);
            size
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdin.as_mut().unwrap().flush()
    }
}

fn stdout_reader(mut gpg: Child, tx: mpsc::Sender<ChunkResult>) -> EmptyResult {
    let mut stderr = gpg.stderr.take().unwrap();
    let mut stderr_reader = Some(thread::Builder::new().name("gpg stderr reader".into()).spawn(move || -> EmptyResult {
        let mut error = String::new();

        match stderr.read_to_string(&mut error) {
            Ok(size) => {
                if size == 0 {
                    Ok(())
                } else {
                    Err!("gpg error: {}", error.trim_right())
                }
            },
            Err(err) => Err!("gpg stderr reading error: {}", err),
        }
    }).map_err(|err| format!("Unable to spawn a thread: {}", err))?);

    let stdout = BufReader::new(gpg.stdout.take().unwrap());

    read_data(stdout, tx).map_err(|err| {
        terminate_gpg(gpg.id() as i32);
        util::join_thread_ignoring_result(stderr_reader.take().unwrap());
        err
    })?;

    util::join_thread(stderr_reader.take().unwrap())?;

    let status = gpg.wait().map_err(|e| format!("Failed to wait() a child gpg process: {}", e))?;
    if status.success() {
        debug!("gpg process has terminated with successful exit code.")
    } else {
        return Err!("gpg process has terminated with an error exit code")
    }

    Ok(())
}

fn read_data(mut stdout: BufReader<ChildStdout>, mut tx: mpsc::Sender<ChunkResult>) -> EmptyResult {
    // FIXME
    let mut out = File::create("backup-mock.tar").unwrap();

    loop {
        let size = {
            let data = stdout.fill_buf().map_err(|e| format!("gpg stdout reading error: {}", e))?;
            if data.len() == 0 {
                break;
            }

            trace!("Got {} bytes of encrypted data.", data.len());
            out.write_all(data)?;

            tx = tx.send(Ok(From::from(data.to_vec()))).wait()?;

            data.len()
        };
        stdout.consume(size);
    }

    Ok(())
}

fn terminate_gpg(pid: i32) {
    let termination_timeout = time::Duration::from_secs(3);
    if let Err(err) = util::terminate_process("a child gpg process", pid, termination_timeout) {
        error!("{}.", err)
    }
}