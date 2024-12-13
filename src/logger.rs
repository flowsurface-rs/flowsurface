use chrono::Local;
use std::{
    fs::{self, File},
    process,
};

const MAX_LOG_FILE_SIZE: u64 = 10_000_000; // 10 MB

pub fn setup(is_debug: bool, log_trace: bool) -> Result<(), fern::InitError> {
    let log_level = if log_trace {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Info
    };

    let mut logger = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}:{} [{}:{}] -- {}",
                Local::now().format("%H:%M:%S%.3f"),
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                message
            ));
        })
        .level(log_level);

    if is_debug {
        logger = logger.chain(std::io::stdout());
    } else {
        let log_file_path = "output.log";
        let log_file = File::create(log_file_path)?;
        log_file.set_len(0)?;

        std::thread::spawn(move || {
            monitor_file_size(log_file_path, MAX_LOG_FILE_SIZE);
        });

        let log_file = fern::log_file(log_file_path)?;
        logger = logger.chain(log_file);
    }

    logger.apply()?;
    Ok(())
}

fn monitor_file_size(file_path: &str, max_size_bytes: u64) {
    loop {
        match fs::metadata(file_path) {
            Ok(metadata) => {
                if metadata.len() > max_size_bytes {
                    eprintln!(
                        "Things went south. Log file size caused panic exceeding {} MB",
                        metadata.len() / 1_000_000,
                    );
                    process::exit(1);
                }
            }
            Err(err) => {
                eprintln!("Error reading log file metadata: {err}");
                process::exit(1);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
