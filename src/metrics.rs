use std::io::{BufWriter, Write};
use std::fs::{self, File};

use prometheus::{self, TextEncoder, Encoder};

use crate::core::{EmptyResult, GenericError};

pub fn save(path: &str) -> EmptyResult {
    let encoder = TextEncoder::new();
    let metrics = prometheus::gather();

    let temp_path = format!("{}.tmp", path);
    let mut file = BufWriter::new(File::create(&temp_path)?);

    encoder.encode(&metrics, &mut file)
        .map_err(Into::into)
        .and_then(|_| {
            Ok(file.flush()?)
        })
        .or_else(|err: GenericError| {
            fs::remove_file(&temp_path)?;
            Err(err)
        })?;

    Ok(fs::rename(&temp_path, path)?)
}