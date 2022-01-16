use std::fmt::{self, Display, Debug, Formatter};
use std::io::{self, Write};

use digest::Digest;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Hash(Vec<u8>);

impl From<&[u8]> for Hash {
    fn from(hash: &[u8]) -> Self {
        Hash(hash.to_vec())
    }
}

impl Display for Hash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        static CHARS: &[u8; 16] = b"0123456789abcdef";

        let mut data = Vec::with_capacity(self.0.len() * 2);
        for &byte in &self.0 {
            data.push(CHARS[(byte >> 4) as usize]);
            data.push(CHARS[(byte & 0xF) as usize]);
        }

        let string = std::str::from_utf8(data.as_slice()).unwrap();
        Display::fmt(string, f)
    }
}

impl Debug for Hash {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

pub trait Hasher: Write + Send {
    fn finish(self: Box<Self>) -> Hash;
}

pub struct ChunkedSha256 {
    block_size: usize,
    block_hasher: Option<BlockHasher>,
    result_hasher: sha2::Sha256,
}

impl ChunkedSha256 {
    pub fn new(block_size: usize) -> ChunkedSha256 {
        ChunkedSha256 {
            block_size,
            block_hasher: None,
            result_hasher: sha2::Sha256::default(),
        }
    }

    fn consume_block(&mut self) {
        if let Some(block_hasher) = self.block_hasher.take() {
            self.result_hasher.update(block_hasher.hasher.finalize().as_slice());
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
                    hasher: sha2::Sha256::new(),
                    available_size,
                });

                available_size
            }
        };

        let consumed_size = if data_size < available_size {
            let block_hasher = self.block_hasher.as_mut().unwrap();
            block_hasher.hasher.update(buf);
            block_hasher.available_size -= data_size;
            data_size
        } else {
            self.block_hasher.as_mut().unwrap().hasher.update(&buf[..available_size]);
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
    fn finish(mut self: Box<Self>) -> Hash {
        self.consume_block();
        self.result_hasher.finalize().as_slice().into()
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
        Md5 {hasher: md5::Md5::new()}
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
    fn finish(self: Box<Self>) -> Hash {
        self.hasher.finalize().as_slice().into()
    }
}