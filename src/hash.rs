use std::io;

use sha2::{self, Digest};

pub trait Hasher: io::Write + Send {
    fn finish(self: Box<Self>) -> String;
}

pub struct ChunkedSha256 {
    block_size: usize,
    available_size: usize,

    block_hasher: sha2::Sha256,
    result_hasher: sha2::Sha256,
}

impl ChunkedSha256 {
    pub fn new(block_size: usize) -> ChunkedSha256 {
        ChunkedSha256 {
            block_size: block_size,
            available_size: block_size,
            block_hasher: sha2::Sha256::default(),
            result_hasher: sha2::Sha256::default(),
        }
    }

    fn consume_block(&mut self) {
        self.result_hasher.input(self.block_hasher.result().as_slice());
        self.block_hasher = sha2::Sha256::default();
        self.available_size = self.block_size;
    }
}

impl Hasher for ChunkedSha256 {
    fn finish(mut self: Box<Self>) -> String {
        if self.available_size != self.block_size {
            self.consume_block();
        }

        format!("{:x}", self.result_hasher.result())
    }
}

impl io::Write for ChunkedSha256 {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let data_size = buf.len();
        let available_size = self.available_size;

        let consumed_size = if data_size < available_size {
            self.block_hasher.input(buf);
            self.available_size -= data_size;
            data_size
        } else {
            self.block_hasher.input(&buf[..available_size]);
            self.consume_block();
            available_size
        };

        Ok(consumed_size)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}