use std::error::Error;
use std::fmt;
use std::io;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use bytes::Bytes;
use futures::{Future, Sink};
use futures::sync::mpsc as futures_mpsc;
use hyper::{self, Chunk};

use core::{EmptyResult, GenericResult};

// FIXME: naming
#[derive(Debug)]
pub enum Data {
    Payload(Bytes),
    EofWithChecksum(String),
}

pub type DataSender = mpsc::SyncSender<GenericResult<Data>>;
pub type DataReceiver = mpsc::Receiver<GenericResult<Data>>;

// FIXME: naming
pub enum ChunkStream {
    Stream(u64, ChunkReceiver),
    EofWithCheckSum(u64, String),
    Error(String),
}

pub type ChunkStreamSender = mpsc::SyncSender<ChunkStream>;
pub type ChunkStreamReceiver = mpsc::Receiver<ChunkStream>;

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
    let mut offset: u64 = 0;

    let mut stream_size: u64 = 0;
    let (mut tx, rx) = futures_mpsc::channel(0);
    chunk_streams.send(ChunkStream::Stream(offset, rx))?;

    for data_result in data_stream.iter() {
        let mut data = match data_result {
            Ok(Data::Payload(data)) => data,
            Ok(Data::EofWithChecksum(checksum)) => {
                // FIXME
                drop(tx);
                chunk_streams.send(ChunkStream::EofWithCheckSum(offset, checksum))?;

                // FIXME
                // Ensure that this error result is the last in the stream and we aren't skipping
                // any data.
//                data_stream.recv().unwrap_err();

                return Ok(());
            },
            Err(err) => {
                // FIXME: https://github.com/tokio-rs/tokio-proto/blob/42ddd45cd34fde8ddd12bdf49a8147762787bf33/src/streaming/pipeline/advanced.rs#L329
                // FIXME
                drop(tx);
                chunk_streams.send(ChunkStream::Error(err.to_string()))?;

                // FIXME
//                let err = io::Error::new(io::ErrorKind::Other, err.to_string()).into();
//                tx.send(Err(err)).wait()?;

                // Ensure that this error result is the last in the stream and we aren't skipping
                // any data.
//                data_stream.recv().unwrap_err();

                return Ok(());
            }
        };

        loop {
            let available_size = stream_max_size - stream_size;
            let data_size = data.len() as u64;

            if available_size >= data_size {
                if data_size > 0 {
                    tx = tx.send(Ok(data.into())).wait()?;
                    stream_size += data_size;
                    offset += data_size;
                }

                break;
            }

            if available_size > 0 {
                tx.send(Ok(data.slice_to(available_size as usize).into())).wait()?;
                data = data.slice_from(available_size as usize);
                offset += available_size;
            }

            let (new_tx, new_rx) = futures_mpsc::channel(0);
            tx = new_tx;
            chunk_streams.send(ChunkStream::Stream(offset, new_rx))?;
            stream_size = 0;
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct StreamSplitterError(String);

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
        StreamSplitterError("Unable to send a new stream: the receiver has been closed".to_owned())
    }
}

impl<T> From<futures_mpsc::SendError<T>> for StreamSplitterError {
    fn from(_err: futures_mpsc::SendError<T>) -> StreamSplitterError {
        StreamSplitterError("Unable to send a new chunk: the receiver has been closed".to_owned())
    }
}