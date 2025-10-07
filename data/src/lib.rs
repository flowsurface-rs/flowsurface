pub mod aggr;
pub mod audio;
pub mod chart;
pub mod config;
pub mod layout;
pub mod log;
pub mod panel;
pub mod tickers_table;
pub mod util;

use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;

pub use audio::AudioStream;
pub use config::ScaleFactor;
pub use config::sidebar::{self, Sidebar};
pub use config::state::{Layouts, State};
pub use config::theme::Theme;
pub use config::timezone::UserTimezone;

use ::log::{error, info, warn};
pub use layout::{Dashboard, Layout, Pane};

pub const SAVED_STATE_PATH: &str = "saved-state.json";

#[derive(thiserror::Error, Debug, Clone)]
pub enum InternalError {
    #[error("Fetch error: {0}")]
    Fetch(String),
    #[error("Layout error: {0}")]
    Layout(String),
}

pub fn write_json_to_file(json: &str, file_name: &str) -> std::io::Result<()> {
    let path = data_path(Some(file_name));
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

pub fn read_from_file(file_name: &str) -> Result<State, Box<dyn std::error::Error>> {
    let path = data_path(Some(file_name));

    let file_open_result = File::open(&path);
    let mut file = match file_open_result {
        Ok(file) => file,
        Err(e) => return Err(Box::new(e)),
    };

    let mut contents = String::new();
    if let Err(e) = file.read_to_string(&mut contents) {
        return Err(Box::new(e));
    }

    match serde_json::from_str(&contents) {
        Ok(state) => Ok(state),
        Err(e) => {
            // If parsing fails, backup the file
            drop(file); // Close the file before renaming

            // Create backup file with different name to prevent overwriting it
            let backup_file_name = if let Some(pos) = file_name.rfind('.') {
                format!("{}_old{}", &file_name[..pos], &file_name[pos..])
            } else {
                format!("{}_old", file_name)
            };

            let backup_path = data_path(Some(&backup_file_name));

            if let Err(rename_err) = std::fs::rename(&path, &backup_path) {
                warn!(
                    "Failed to backup corrupted state file '{}' to '{}': {}",
                    path.display(),
                    backup_path.display(),
                    rename_err
                );
            } else {
                info!(
                    "Backed up corrupted state file to '{}'. It can be restored manually.",
                    backup_path.display()
                );
            }

            Err(Box::new(e))
        }
    }
}

pub fn open_data_folder() -> Result<(), InternalError> {
    let pathbuf = data_path(None);

    if pathbuf.exists() {
        if let Err(err) = open::that(&pathbuf) {
            Err(InternalError::Layout(format!(
                "Failed to open data folder: {:?}, error: {}",
                pathbuf, err
            )))
        } else {
            info!("Opened data folder: {:?}", pathbuf);
            Ok(())
        }
    } else {
        Err(InternalError::Layout(format!(
            "Data folder does not exist: {:?}",
            pathbuf
        )))
    }
}

pub fn data_path(path_name: Option<&str>) -> PathBuf {
    if let Ok(path) = std::env::var("FLOWSURFACE_DATA_PATH") {
        PathBuf::from(path)
    } else {
        let data_dir = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
        if let Some(path_name) = path_name {
            data_dir.join("flowsurface").join(path_name)
        } else {
            data_dir.join("flowsurface")
        }
    }
}

fn cleanup_directory(data_path: &PathBuf) -> usize {
    if !data_path.exists() {
        warn!("Data path {:?} does not exist, skipping cleanup", data_path);
        return 0;
    }

    let re =
        regex::Regex::new(r".*-(\d{4}-\d{2}-\d{2})\.zip$").expect("Cleanup regex pattern is valid");
    let today = chrono::Local::now().date_naive();
    let mut deleted_files = Vec::new();

    let entries = match std::fs::read_dir(data_path) {
        Ok(entries) => entries,
        Err(e) => {
            error!("Failed to read data directory {:?}: {}", data_path, e);
            return 0;
        }
    };

    for entry in entries.filter_map(Result::ok) {
        let symbol_dir = match std::fs::read_dir(entry.path()) {
            Ok(dir) => dir,
            Err(e) => {
                error!("Failed to read symbol directory {:?}: {}", entry.path(), e);
                continue;
            }
        };

        for file in symbol_dir.filter_map(Result::ok) {
            let path = file.path();
            let Some(filename) = path.to_str() else {
                continue;
            };

            if let Some(cap) = re.captures(filename)
                && let Ok(file_date) = chrono::NaiveDate::parse_from_str(&cap[1], "%Y-%m-%d")
            {
                let days_old = today.signed_duration_since(file_date).num_days();
                if days_old > 4 {
                    if let Err(e) = std::fs::remove_file(&path) {
                        error!("Failed to remove old file {}: {}", filename, e);
                    } else {
                        deleted_files.push(filename.to_string());
                        info!("Removed old file: {}", filename);
                    }
                }
            }
        }
    }

    deleted_files.len()
}

pub fn cleanup_old_market_data() -> usize {
    let paths = ["um", "cm"].map(|market_type| {
        data_path(Some(&format!(
            "market_data/binance/data/futures/{}/daily/aggTrades",
            market_type
        )))
    });

    let total_deleted: usize = paths.iter().map(cleanup_directory).sum();

    info!("File cleanup completed. Deleted {} files", total_deleted);
    total_deleted
}
