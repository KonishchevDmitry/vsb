use std::io::{BufWriter, Write};
use std::fs::{self, File};

use prometheus::{self, TextEncoder, Encoder, GaugeVec};

use crate::core::{EmptyResult, GenericError};
use crate::storage::BackupGroup;

lazy_static! {
    static ref EXTERN_FILES: GaugeVec = register(
        "extern_files", "Number of extern files in the last backup.");
    static ref UNIQUE_FILES: GaugeVec = register(
        "unique_files", "Number of unique files in the last backup.");
    static ref ERROR_FILES: GaugeVec = register(
        "error_files", "Number of missing files in the last backup.");
}

pub fn collect(name: &str, groups: &[BackupGroup]) -> EmptyResult {
    let mut last_backup = None;

    for group in groups.iter().rev() {
        if let Some(backup) = group.backups.last() {
            last_backup.replace(backup);
            break;
        }
    }

    let stat = match last_backup {
        Some(backup) => {
            match backup.stat.as_ref() {
                Some(stat) => stat,
                None => return Err!("The backup has no collected statistics"),
            }
        }
        None => return Ok(()),
    };

    EXTERN_FILES.with_label_values(&[name]).set(stat.extern_files as f64);
    UNIQUE_FILES.with_label_values(&[name]).set(stat.unique_files as f64);
    ERROR_FILES.with_label_values(&[name]).set(stat.error_files as f64);

    Ok(())
}

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

fn register(name: &str, help: &str) -> GaugeVec {
    register_gauge_vec!(&format!("backup_{}", name), help, &["name"]).unwrap()
}