use std::io::{self, Read};

use digest::Digest as DigestTrait;
use lazy_static::lazy_static;

use crate::util::hash::Hash;

type Digest = sha2::Sha512;

lazy_static! {
    pub static ref EMPTY_FILE_HASH: Hash = Digest::new().finalize().as_slice().into();
}

pub struct FileReader<'a> {
    file: &'a mut dyn Read,
    digest: Digest,
    bytes_read: u64,
    bytes_left: u64,
    truncated: bool,
}

impl<'a> FileReader<'a> {
    pub fn new(file: &mut dyn Read, size: u64) -> FileReader {
        FileReader {
            file,
            digest: Digest::new(),
            bytes_read: 0,
            bytes_left: size,
            truncated: false,
        }
    }

    pub fn consume(self) -> (u64, Hash) {
        let hash = self.digest.finalize().as_slice().into();
        (self.bytes_read, hash)
    }
}

impl<'a> Read for FileReader<'a> {
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        if self.bytes_left < buf.len() as u64 {
            buf = &mut buf[..self.bytes_left as usize]
        }
        if buf.is_empty() {
            return Ok(0);
        }

        // We have to write the exact file size to not corrupt the archive
        if self.truncated {
            buf.fill(0);
            let size = buf.len();
            self.bytes_left -= size as u64;
            return Ok(size);
        }

        let size = self.file.read(buf)?;
        if size == 0 {
            self.truncated = true;
            return self.read(buf);
        }

        self.bytes_read += size as u64;
        self.bytes_left -= size as u64;
        self.digest.update(&buf[..size]);

        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Buf;
    use rand::{Rng, RngCore};
    use rayon::prelude::*;
    use super::*;

    #[test]
    fn file_reader() {
        let mut random = rand::thread_rng();

        let mut data = vec![0_u8; 1024 * 1024];
        random.fill_bytes(&mut data);

        let file_sizes: Vec<usize> =
            [0, data.len()].into_iter()
            .chain(std::iter::repeat_with(|| random.gen_range(1..data.len())).take(10))
            .collect();

        let test = |file_mock: &[u8], file_size: usize| {
            let expected_hash: Hash = Digest::digest(file_mock).as_slice().into();

            let mut result_data: Vec<u8> = Vec::with_capacity(file_size);
            let expected_data: Vec<u8> = file_mock.iter().cloned()
                .chain(std::iter::repeat(0).take(file_size - file_mock.len())).collect();

            let mut reader = file_mock.reader();
            let mut file_reader = FileReader::new(&mut reader, file_size as u64);
            io::copy(&mut file_reader, &mut result_data).unwrap();
            let (bytes_read, hash) = file_reader.consume();

            assert_eq!(bytes_read, file_mock.len() as u64);
            assert_eq!(hash, expected_hash);

            if result_data != expected_data {
                panic!("Got an invalid data")
            }
        };

        file_sizes.into_par_iter().for_each(|file_size| {
            let file_mock = &data[..file_size];
            [0, rand::thread_rng().gen_range(1..=data.len())].into_par_iter().for_each(|truncated_size| {
                test(file_mock, file_size + truncated_size);
            })
        })
    }
}