use std::io;
use std::sync::mpsc;

use bytes::{Buf, Bytes};

use crate::core::GenericResult;

type Message = Result<Bytes, String>;
type ChunkStream = mpsc::Receiver<Message>;

pub enum Body {
    String(String),
    Stream(ChunkStream),
}

impl<'a> From<&'a str> for Body {
    fn from(data: &str) -> Self {
        Body::String(data.to_owned())
    }
}

impl From<String> for Body {
    fn from(data: String) -> Self {
        Body::String(data)
    }
}

impl From<ChunkStream> for Body {
    fn from(stream: ChunkStream) -> Self {
        Body::Stream(stream)
    }
}

impl From<Body> for reqwest::blocking::Body {
    fn from(body: Body) -> Self {
        match body {
            Body::String(data) => data.into(),
            Body::Stream(stream) => reqwest::blocking::Body::new(StreamReader {
                stream: stream,
                current_chunk: None,
            })
        }
    }
}

struct StreamReader {
    stream: ChunkStream,
    current_chunk: Option<Bytes>,
}

impl StreamReader {
    fn get_current_chunk(&mut self) -> GenericResult<Option<&mut Bytes>> {
        if self.current_chunk.is_none() {
            let chunk = match self.stream.recv() {
                Ok(message) => message?,
                Err(mpsc::RecvError) => return Ok(None),
            };

            self.current_chunk = Some(chunk);
        }

        Ok(Some(self.current_chunk.as_mut().unwrap()))
    }
}

impl io::Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (empty, size) = {
            let data = self.get_current_chunk().map_err(|e|
                io::Error::new(io::ErrorKind::Other, e))?;

            let data: &mut Bytes = match data {
                Some(data) => data,
                None => return Ok(0),
            };

            let data_size = data.len();
            assert_ne!(data_size, 0);

            let size = std::cmp::min(buf.len(), data_size);
            buf[..size].copy_from_slice(&data[..size]);
            data.advance(size);

            (data.is_empty(), size)
        };

        if empty {
            self.current_chunk.take();
        }

        Ok(size)
    }
}