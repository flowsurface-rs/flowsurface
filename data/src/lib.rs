pub mod aggr;
pub mod chart;
pub mod config;
pub mod layout;

use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;

use chrono::NaiveDate;
pub use config::ScaleFactor;
pub use config::sidebar::{self, Sidebar};
pub use config::state::{Layouts, State};
pub use config::theme::Theme;
pub use config::timezone::UserTimezone;

pub use layout::{Dashboard, Layout, Pane};
use regex::Regex;

#[derive(thiserror::Error, Debug, Clone)]
pub enum InternalError {
    #[error("Fetch error: {0}")]
    Fetch(String),
    #[error("Layout error: {0}")]
    Layout(String),
}

pub fn write_json_to_file(json: &str, file_name: &str) -> std::io::Result<()> {
    let path = get_data_path(file_name);
    let mut file = File::create(path)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

pub fn read_from_file(file_name: &str) -> Result<State, Box<dyn std::error::Error>> {
    let path = get_data_path(file_name);

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

            // Create backup filename with "_old" added
            let backup_path = if let Some(ext) = path.extension() {
                let mut old_filename = path.file_stem().unwrap().to_os_string();
                old_filename.push("_old");
                path.with_file_name(old_filename).with_extension(ext)
            } else {
                let mut old_filename = path.file_name().unwrap().to_os_string();
                old_filename.push("_old");
                path.with_file_name(old_filename)
            };

            if let Err(rename_err) = std::fs::rename(&path, &backup_path) {
                log::warn!(
                    "Failed to backup corrupted state file '{}' to '{}': {}",
                    path.display(),
                    backup_path.display(),
                    rename_err
                );
            } else {
                log::info!(
                    "Backed up corrupted state file to '{}'. It can be restored it manually.",
                    backup_path.display()
                );
            }

            Err(Box::new(e))
        }
    }
}

pub fn get_data_path(path_name: &str) -> PathBuf {
    if let Ok(path) = std::env::var("FLOWSURFACE_DATA_PATH") {
        PathBuf::from(path)
    } else {
        let data_dir = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
        data_dir.join("flowsurface").join(path_name)
    }
}

pub fn cleanup_old_market_data() -> usize {
    let data_path = get_data_path("market_data/binance/data/futures/um/daily/aggTrades");

    if !data_path.exists() {
        log::warn!("Data path {:?} does not exist, skipping cleanup", data_path);
        return 0;
    }

    let re = Regex::new(r".*-(\d{4}-\d{2}-\d{2})\.zip$").expect("Cleanup regex pattern is valid");
    let today = chrono::Local::now().date_naive();
    let mut deleted_files = Vec::new();

    let entries = match std::fs::read_dir(data_path) {
        Ok(entries) => entries,
        Err(e) => {
            log::error!("Failed to read data directory: {}", e);
            return 0;
        }
    };

    for entry in entries.filter_map(Result::ok) {
        let symbol_dir = match std::fs::read_dir(entry.path()) {
            Ok(dir) => dir,
            Err(e) => {
                log::error!("Failed to read symbol directory {:?}: {}", entry.path(), e);
                continue;
            }
        };

        for file in symbol_dir.filter_map(Result::ok) {
            let path = file.path();
            let filename = match path.to_str() {
                Some(name) => name,
                None => continue,
            };

            if let Some(cap) = re.captures(filename) {
                if let Ok(file_date) = NaiveDate::parse_from_str(&cap[1], "%Y-%m-%d") {
                    let days_old = today.signed_duration_since(file_date).num_days();
                    if days_old > 4 {
                        if let Err(e) = std::fs::remove_file(&path) {
                            log::error!("Failed to remove old file {}: {}", filename, e);
                        } else {
                            deleted_files.push(filename.to_string());
                            log::info!("Removed old file: {}", filename);
                        }
                    }
                }
            }
        }
    }

    log::info!(
        "File cleanup completed. Deleted {} files",
        deleted_files.len()
    );
    deleted_files.len()
}
