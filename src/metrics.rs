use std::io::{BufWriter, Write};
use std::fs::{self, File};

use prometheus::{self, TextEncoder, Encoder, GaugeVec};

use crate::core::{EmptyResult, GenericError};
use crate::storage::BackupGroup;

lazy_static! {
    static ref FILES: GaugeVec = register("files", "Number of files in the last backup.");
    static ref FILES_SIZE: GaugeVec = register("files_size", "Files size in the last backup.");

    static ref SIZE: GaugeVec = register("size", "Last backup size.");
    static ref TOTAL_SIZE: GaugeVec = register("total_size", "Total size of all backups.");
}

pub fn collect(name: &str, groups: &[BackupGroup]) -> EmptyResult {
    collect_last_backup(name, groups)?;
    collect_total(name, groups)?;
    Ok(())
}

fn collect_last_backup(name: &str, groups: &[BackupGroup]) -> EmptyResult {
    let mut last_backup = None;

    for group in groups.iter().rev() {
        if let Some(backup) = group.backups.last() {
            last_backup.replace(backup);
            break;
        }
    }

    let (inner_stat, outer_stat) = match last_backup {
        Some(backup) => {
            match (backup.inner_stat.as_ref(), backup.outer_stat.as_ref()) {
                (Some(inner), Some(outer)) => (inner, outer),
                _ => return Err!("The backup has no collected statistics"),
            }
        }
        None => return Ok(()),
    };

    for &(type_, count) in &[
        ("extern", inner_stat.extern_files),
        ("unique", inner_stat.unique_files),
    ] {
        FILES.with_label_values(&[name, type_]).set(count as f64);
    }

    for &(type_, size) in &[
        ("extern", inner_stat.extern_size),
        ("unique", inner_stat.unique_size),
    ] {
        FILES_SIZE.with_label_values(&[name, type_]).set(size as f64);
    }

    for &(type_, size) in &[
        ("metadata", outer_stat.metadata_size),
        ("data", outer_stat.data_size),
    ] {
        SIZE.with_label_values(&[name, type_]).set(size as f64);
    }

    Ok(())
}

fn collect_total(name: &str, groups: &[BackupGroup]) -> EmptyResult {
    let mut metadata_size = 0;
    let mut data_size = 0;

    for group in groups {
        for backup in &group.backups {
            let stat = backup.outer_stat.as_ref().ok_or_else(||
                "The backup has no collected statistics")?;

            metadata_size += stat.metadata_size;
            data_size += stat.data_size;
        }
    }

    for &(type_, size) in &[
        ("metadata", metadata_size),
        ("data", data_size),
    ] {
        TOTAL_SIZE.with_label_values(&[name, type_]).set(size as f64);
    }

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
    register_gauge_vec!(&format!("backup_{}", name), help, &["name", "type"]).unwrap()
}