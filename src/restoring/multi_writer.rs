use std::fs::File;
use std::io::Write;

pub struct MultiWriter {
    files: Vec<File>,
}

impl MultiWriter {
    pub fn new(files: Vec<File>) -> MultiWriter {
        MultiWriter {files}
    }
}

impl Write for MultiWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for file in &mut self.files {
            file.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        for file in &mut self.files {
            file.flush()?;
        }
        Ok(())
    }
}