use std::io::{Read, BufRead, BufReader, Lines, Write, BufWriter};

use bzip2::Compression;
use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;

use crate::core::{EmptyResult, GenericResult};

pub struct MetadataItem {
    pub path: String,
    pub size: u64,
    pub unique: bool,
    pub checksum: String,
}

pub struct MetadataReader {
    lines: Lines<Box<dyn BufRead>>,
}

impl MetadataReader {
    pub fn new<R: Read + 'static>(reader: R) -> MetadataReader {
        let reader: Box<dyn BufRead> = Box::new(BufReader::new(BzDecoder::new(reader)));
        MetadataReader {lines: reader.lines()}
    }
}

impl Iterator for MetadataReader {
    type Item = GenericResult<MetadataItem>;

    fn next(&mut self) -> Option<GenericResult<MetadataItem>> {
        self.lines.next().map(|line| {
            let line = line?;

            let mut parts = line.splitn(4, ' ');
            let checksum = parts.next();
            let status = parts.next();
            let fingerprint = parts.next();
            let path = parts.next();

            let (checksum, unique, fingerprint, path) = match (checksum, status, fingerprint, path) {
                (Some(checksum), Some(status), Some(fingerprint), Some(filename))
                if status == "extern" || status == "unique" => (
                    checksum, status == "unique", fingerprint, filename,
                ),
                _ => return Err!("Unexpected format: {:?}", line),
            };

            let size = fingerprint.rsplit(':').next().unwrap();
            let size: u64 = size.parse().map_err(|_| format!("Invalid file size: {:?}", size))?;

            Ok(MetadataItem {
                path: path.to_owned(),
                checksum: checksum.to_owned(),
                size, unique
            })
        })
    }
}

// FIXME(konishchev): flush + fsync
pub struct MetadataWriter {
    writer: Box<dyn Write>,
}

impl MetadataWriter {
    pub fn new<W: Write + 'static>(writer: W) -> MetadataWriter {
        let writer = BzEncoder::new(writer, Compression::best());
        MetadataWriter {
            writer: Box::new(BufWriter::new(writer))
        }
    }

    // FIXME(konishchev): Validate params
    #[allow(dead_code)] // FIXME(konishchev): Drop
    pub fn write(&mut self, item: &MetadataItem) -> EmptyResult {
        let status = match item.unique {
            true => "unique",
            false => "extern",
        };
        Ok(write!(&mut self.writer, "{} {} {} {}", item.checksum, status, item.size, item.path)?)
    }
}