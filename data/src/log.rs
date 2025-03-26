use std::path::PathBuf;
use std::{fs, io};

use crate::get_data_path;

const LOG_FILE: &str = "flowsurface-current.log";

pub fn file() -> Result<fs::File, Error> {
    let path = path()?;

    Ok(fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(false)
        .truncate(true)
        .open(path)?)
}

pub fn path() -> Result<PathBuf, Error> {
    let full_path = get_data_path(LOG_FILE);

    let parent = full_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid log file path"))?;

    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    Ok(full_path)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    SetLog(#[from] log::SetLoggerError),
    #[error(transparent)]
    ParseLevel(#[from] log::ParseLevelError),
}
