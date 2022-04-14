use std::fs;
use std::io::{self, Read, BufRead, BufReader, Lines, Write, BufWriter};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bzip2::Compression;
use bzip2::read::BzDecoder;
use bzip2::write::BzEncoder;

use crate::core::{EmptyResult, GenericResult};
use crate::util::hash::Hash;

pub struct MetadataItem {
    pub path: String,
    pub size: u64,
    pub hash: Hash,
    pub unique: bool,
    pub fingerprint: Fingerprint,
}

impl MetadataItem {
    pub fn new(path: &Path, size: u64, hash: Hash, fingerprint: Fingerprint, unique: bool) -> GenericResult<MetadataItem> {
        let path = validate_path(path)?.to_owned();
        Ok(MetadataItem {path, size, hash, unique, fingerprint})
    }

    fn encode(&self, writer: &mut dyn Write) -> EmptyResult {
        let status = match self.unique {
            true => "unique",
            false => "extern",
        };

        Ok(writeln!(
            writer, "{status} {hash} {fingerprint} {size} {path}",
            status=status, hash=self.hash, fingerprint=self.fingerprint.encode(), size=self.size,
            path=self.path,
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

        let hash = parts.next().ok_or_else(error)?.try_into()?;
        let fingerprint = parts.next().and_then(Fingerprint::decode).ok_or_else(error)?;

        let size = parts.next().and_then(|v| v.parse::<u64>().ok()).ok_or_else(error)?;
        let path = parts.next().ok_or_else(error)?.to_owned();

        Ok(MetadataItem {path, size, hash, unique, fingerprint})
    }
}

#[derive(Debug, PartialEq)]
pub struct Fingerprint {
    device: u64,
    inode: u64,
    mtime_nsec: i128,
}

impl Fingerprint {
    pub fn new(metadata: &fs::Metadata) -> Fingerprint {
        Fingerprint {
            device: metadata.dev(),
            inode: metadata.ino(),
            mtime_nsec: metadata.mtime() as i128 * 1_000_000_000 + metadata.mtime_nsec() as i128,
        }
    }

    fn encode(&self) -> String {
        format!(
            "{device}:{inode}:{mtime}",
            device=self.device, inode=self.inode, mtime=self.mtime_nsec
        )
    }

    fn decode(fingerprint: &str) -> Option<Fingerprint> {
        let mut parts = fingerprint.split(':');

        let device = parts.next().and_then(|v| v.parse::<u64>().ok());
        let inode = parts.next().and_then(|v| v.parse::<u64>().ok());
        let mtime_nsec = parts.next().and_then(|v| v.parse::<i128>().ok());

        match (device, inode, mtime_nsec, parts.next()) {
            (Some(device), Some(inode), Some(mtime_nsec), None) => Some(Fingerprint {
                device, inode, mtime_nsec,
            }),
            _ => None,
        }
    }
}

pub fn validate_path(path: &Path) -> GenericResult<&str> {
    Ok(path.to_str().and_then(|path: &str| {
        for byte in path.bytes() {
            if byte == b'\r' || byte == b'\n' {
                return None;
            }
        }
        Some(path)
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

pub struct MetadataWriter<W: Write> {
    writer: BufWriter<BzEncoder<W>>,
}

impl<W: Write> MetadataWriter<W> {
    pub fn new(writer: W) -> MetadataWriter<W> {
        MetadataWriter {
            writer: BufWriter::new(BzEncoder::new(writer, Compression::best()))
        }
    }

    pub fn write(&mut self, item: &MetadataItem) -> EmptyResult {
        item.encode(&mut self.writer)
    }

    pub fn finish(self) -> io::Result<W> {
        self.writer.into_inner()?.finish()
    }
}