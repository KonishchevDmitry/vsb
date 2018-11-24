use std::io::{self, Write};

use digest::FixedOutput;
use md5;
use sha2::{self, Digest};

pub trait Hasher: Write + Send {
    fn finish(self: Box<Self>) -> String;
}

pub struct ChunkedSha256 {
    block_size: usize,
    block_hasher: Option<BlockHasher>,
    result_hasher: sha2::Sha256,
}

impl ChunkedSha256 {
    pub fn new(block_size: usize) -> ChunkedSha256 {
        ChunkedSha256 {
            block_size: block_size,
            block_hasher: None,
            result_hasher: sha2::Sha256::default(),
        }
    }

    fn consume_block(&mut self) {
        if let Some(block_hasher) = self.block_hasher.take() {
            self.result_hasher.input(block_hasher.hasher.result().as_slice());
        }
    }
}

impl Write for ChunkedSha256 {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let data_size = buf.len();
        if data_size == 0 {
            return Ok(data_size)
        }

        let available_size = match self.block_hasher {
            Some(ref block_hasher) => block_hasher.available_size,
            None => {
                let available_size = self.block_size;

                self.block_hasher = Some(BlockHasher {
                    hasher: sha2::Sha256::default(),
                    available_size: available_size,
                });

                available_size
            }
        };

        let consumed_size = if data_size < available_size {
            let block_hasher = self.block_hasher.as_mut().unwrap();
            block_hasher.hasher.input(buf);
            block_hasher.available_size -= data_size;
            data_size
        } else {
            self.block_hasher.as_mut().unwrap().hasher.input(&buf[..available_size]);
            self.consume_block();
            available_size
        };

        Ok(consumed_size)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Hasher for ChunkedSha256 {
    fn finish(mut self: Box<Self>) -> String {
        self.consume_block();
        format!("{:x}", self.result_hasher.result())
    }
}

struct BlockHasher {
    hasher: sha2::Sha256,
    available_size: usize,
}

pub struct Md5 {
    hasher: md5::Md5,
}

impl Md5 {
    pub fn new() -> Md5 {
        Md5 {hasher: md5::Md5::default()}
    }
}

impl Write for Md5 {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.hasher.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.hasher.flush()
    }
}

impl Hasher for Md5 {
    fn finish(self: Box<Self>) -> String {
        format!("{:x}", self.hasher.fixed_result())
    }
}