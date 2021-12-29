use std::fs;
use std::io::{Read, BufRead, BufReader, Lines, Write, BufWriter};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bzip2::Compression;
use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;

use crate::core::{EmptyResult, GenericResult};
use crate::hash::Hash;

pub struct MetadataItem {
    pub path: String,
    pub size: u64,
    pub hash: Hash,
    pub unique: bool,

    device: u64,
    inode: u64,
    mtime_nsec: i128,
}

impl MetadataItem {
    pub fn new(path: &Path, metadata: &fs::Metadata, size: u64, hash: Hash, unique: bool) -> GenericResult<MetadataItem> {
        let path = validate_path(path)?.to_owned();

        Ok(MetadataItem {
            path, size, hash, unique,

            device: metadata.dev(),
            inode: metadata.ino(),
            mtime_nsec: metadata.mtime() as i128 * 1_000_000_000 + metadata.mtime_nsec() as i128,
        })
    }

    fn encode(&self, writer: &mut dyn Write) -> EmptyResult {
        let status = match self.unique {
            true => "unique",
            false => "extern",
        };

        Ok(writeln!(
            writer, "{status} {hash} {device}:{inode}:{mtime} {size} {path}",
            status=status, hash=self.hash, device=self.device, inode=self.inode,
            mtime=self.mtime_nsec, size=self.size, path=self.path,
        )?)
    }

    fn decode(line: &str) -> GenericResult<MetadataItem> {
        let mut parts = line.splitn(5, ' ');
        let error = || format!("Unexpected format: {:?}", line);

        let unique = parts.next().and_then(|status| match status {
            "extern" => Some(false),
            "unique" => Some(true),
            _ => None,
        }).ok_or_else(error)?;

        let hash = parts.next().ok_or_else(error)?.as_bytes().into();
        let (device, inode, mtime_nsec) = parts.next().and_then(|fingerprint: &str| {
            let mut parts = fingerprint.split(':');

            let device = parts.next().and_then(|v| v.parse::<u64>().ok());
            let inode = parts.next().and_then(|v| v.parse::<u64>().ok());
            let mtime = parts.next().and_then(|v| v.parse::<i128>().ok());

            match (device, inode, mtime, parts.next()) {
                (Some(device), Some(inode), Some(mtime), None) => Some((device, inode, mtime)),
                _ => None,
            }
        }).ok_or_else(error)?;

        let size = parts.next().and_then(|v| v.parse::<u64>().ok()).ok_or_else(error)?;
        let path = parts.next().ok_or_else(error)?.to_owned();

        Ok(MetadataItem {
            path, hash, unique,
            device, inode, mtime_nsec,
            size,
        })
    }
}

pub fn validate_path(path: &Path) -> GenericResult<&str> {
    Ok(path.to_str().and_then(|path: &str| {
        if path.contains('\n') {
            None
        } else {
            Some(path)
        }
    }).ok_or("invalid path")?)
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
        self.lines.next().map(|line| MetadataItem::decode(&line?))
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

    pub fn write(&mut self, item: &MetadataItem) -> EmptyResult {
        item.encode(&mut self.writer)
    }
}