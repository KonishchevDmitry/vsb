use std::io::{self, Write};
use std::process::{Command, Child, Stdio};

use core::GenericResult;

pub struct Encryptor {
    gpg: Child,
}

impl Encryptor {
    // FIXME
    pub fn new() -> GenericResult<Encryptor> {
//        {
//            // limited borrow of stdin
//            let stdin = child.stdin.as_mut().expect("failed to get stdin");
//            stdin.write_all(b"test").expect("failed to write to stdin");
//        }
//
//        let output = child
//            .wait_with_output()
//            .expect("failed to wait on child");
//
//        assert_eq!(b"testd", output.stdout.as_slice());

        Ok(Encryptor {
            gpg: Command::new("grep").arg("a")
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                .spawn()?
        })
    }
}

impl Drop for Encryptor {
    fn drop(&mut self) {
        // FIXME
    }
}

impl io::Write for Encryptor {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}