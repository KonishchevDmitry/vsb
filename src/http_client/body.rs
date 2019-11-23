use std::io;
use std::sync::mpsc;

use bytes::Buf;
use hyper::{self, Chunk};
use reqwest;

use core::GenericResult;

type Message = Result<Chunk, hyper::Error>;
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

impl Into<reqwest::Body> for Body {
    fn into(self) -> reqwest::Body {
        match self {
            Body::String(data) => data.into(),
            Body::Stream(stream) => reqwest::Body::new(StreamReader {
                stream: stream,
                current_chunk: None,
            })
        }
    }
}

struct StreamReader {
    stream: ChunkStream,
    current_chunk: Option<Chunk>,
}

impl StreamReader {
    fn get_current_chunk(&mut self) -> GenericResult<Option<&mut Chunk>> {
        if self.current_chunk.is_none() {
            let message: Message = match self.stream.recv() {
                Ok(message) => message,
                Err(mpsc::RecvError) => return Ok(None),
            };

            self.current_chunk = Some(message?);
        }

        Ok(Some(self.current_chunk.as_mut().unwrap()))
    }
}

impl io::Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (empty, size) = {
            let chunk = self.get_current_chunk().map_err(|e|
                io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            let chunk = match chunk {
                Some(chunk) => chunk,
                None => return Ok(0),
            };

            let chunk_size = chunk.remaining();
            assert_ne!(chunk_size, 0);

            let size = std::cmp::min(buf.len(), chunk_size);
            buf[..size].copy_from_slice(&chunk.bytes()[..size]);
            chunk.advance(size);

            (chunk.remaining() == 0, size)
        };

        if empty {
            self.current_chunk.take();
        }

        Ok(size)
    }
}