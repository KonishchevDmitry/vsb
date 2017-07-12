use std::error::Error;
use std::fmt;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use bytes::Bytes;
use futures::{Future, Sink};
use futures::sync::mpsc as futures_mpsc;
use hyper::{self, Chunk};

use core::{EmptyResult, GenericResult};

pub enum Data {
    Payload(Bytes),
    EofWithChecksum(String),
}

pub type DataSender = mpsc::SyncSender<Result<Data, String>>;
pub type DataReceiver = mpsc::Receiver<Result<Data, String>>;

pub enum ChunkStream {
    Stream(u64, ChunkReceiver),
    EofWithCheckSum(u64, String),
}

pub type ChunkStreamSender = mpsc::SyncSender<Result<ChunkStream, String>>;
pub type ChunkStreamReceiver = mpsc::Receiver<Result<ChunkStream, String>>;

pub type ChunkReceiver = futures_mpsc::Receiver<ChunkResult>;
pub type ChunkResult = Result<Chunk, hyper::Error>;

pub fn split(data_stream: DataReceiver, stream_max_size: u64) -> GenericResult<(ChunkStreamReceiver, JoinHandle<EmptyResult>)> {
    let (streams_tx, streams_rx) = mpsc::sync_channel(0);

    let splitter_thread = thread::Builder::new().name("stream splitter".into()).spawn(move || {
        Ok(splitter(data_stream, streams_tx, stream_max_size)?)
    }).map_err(|e| format!("Unable to spawn a thread: {}", e))?;

    Ok((streams_rx, splitter_thread))
}

fn splitter(data_stream: DataReceiver, chunk_streams: ChunkStreamSender, stream_max_size: u64) -> Result<(), StreamSplitterError> {
    let mut chunk_stream = None;
    let mut stream_size: u64 = 0;
    let mut offset: u64 = 0;

    loop {
        let message = match data_stream.recv() {
            Ok(message) => message,
            Err(_) => return Err(StreamSplitterError(
                "Unable to receive a new message: the sender has been closed")),
        };

        let mut data = match message {
            Ok(Data::Payload(data)) => data,
            Ok(Data::EofWithChecksum(checksum)) => {
                chunk_stream.take();
                chunk_streams.send(Ok(ChunkStream::EofWithCheckSum(offset, checksum)))?;
                break;
            },
            Err(err) => {
                // Attention:
                // We can't send errors via chunk streams, because it's not supported yet: tokio
                // panics here - https://github.com/tokio-rs/tokio-proto/blob/42ddd45cd34fde8ddd12bdf49a8147762787bf33/src/streaming/pipeline/advanced.rs#L329
                chunk_stream.take();
                chunk_streams.send(Err(err))?;
                break;
            }
        };

        loop {
            let data_size = data.len() as u64;
            if data_size == 0 {
                break;
            }

            if chunk_stream.is_none() {
                let (tx, rx) = futures_mpsc::channel(0);

                chunk_stream = Some(tx);
                stream_size = 0;

                chunk_streams.send(Ok(ChunkStream::Stream(offset, rx)))?;
            }

            let available_size = stream_max_size - stream_size;

            if available_size >= data_size {
                chunk_stream = Some(chunk_stream.unwrap().send(Ok(data.into())).wait()?);
                stream_size += data_size;
                offset += data_size;
                break;
            }

            if available_size > 0 {
                chunk_stream.take().unwrap().send(
                    Ok(data.slice_to(available_size as usize).into())).wait()?;
                data = data.slice_from(available_size as usize);
                stream_size += available_size;
                offset += available_size;
            } else {
                chunk_stream.take();
            }
        }
    }

    if let Ok(_) = data_stream.recv() {
        return Err(StreamSplitterError("Got a message after a termination message"))
    }

    Ok(())
}

#[derive(Debug)]
struct StreamSplitterError(&'static str);

impl Error for StreamSplitterError {
    fn description(&self) -> &str {
        "Stream splitter error"
    }
}

impl fmt::Display for StreamSplitterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T> From<mpsc::SendError<T>> for StreamSplitterError {
    fn from(_err: mpsc::SendError<T>) -> StreamSplitterError {
        StreamSplitterError("Unable to send a new stream: the receiver has been closed")
    }
}

impl<T> From<futures_mpsc::SendError<T>> for StreamSplitterError {
    fn from(_err: futures_mpsc::SendError<T>) -> StreamSplitterError {
        StreamSplitterError("Unable to send a new chunk: the receiver has been closed")
    }
}