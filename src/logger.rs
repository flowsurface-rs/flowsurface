use std::{
    backtrace::Backtrace,
    fs,
    io::{self, Write},
    panic::PanicHookInfo,
    path::PathBuf,
    process,
    sync::{Once, mpsc},
    thread::{self, JoinHandle},
};

const MAX_LOG_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB

enum LogMessage {
    Content(Vec<u8>),
    Flush,
    Shutdown,
}

pub fn setup(is_debug: bool) -> Result<(), data::log::Error> {
    let default_level = if is_debug {
        log::Level::Debug
    } else {
        log::Level::Info
    };

    let level_filter = std::env::var("RUST_LOG")
        .ok()
        .as_deref()
        .map(str::parse::<log::Level>)
        .transpose()?
        .unwrap_or(default_level)
        .to_level_filter();

    let mut io_sink = fern::Dispatch::new().format(|out, message, record| {
        out.finish(format_args!(
            "{}:{} -- {}",
            chrono::Local::now().format("%H:%M:%S%.3f"),
            record.level(),
            message
        ));
    });

    if is_debug {
        io_sink = io_sink.chain(std::io::stdout());
    } else {
        let log_path = data::log::path()?;
        initial_rotation(&log_path)?;

        let logger: Box<dyn Write + Send> = Box::new(BackgroundLogger::new(log_path)?);

        io_sink = io_sink.chain(logger);
    }

    fern::Dispatch::new()
        .level(log::LevelFilter::Off)
        .level_for("panic", log::LevelFilter::Error)
        .level_for("iced_wgpu", log::LevelFilter::Info)
        .level_for("flowsurface_exchange", level_filter)
        .level_for("flowsurface_data", level_filter)
        .level_for("flowsurface", level_filter)
        .chain(io_sink)
        .apply()?;

    Ok(())
}

fn initial_rotation(log_path: &PathBuf) -> io::Result<()> {
    let dir = log_path.parent().unwrap_or(std::path::Path::new("."));
    let previous = dir.join("flowsurface-previous.log");

    if let Err(e) = fs::remove_file(&previous)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e);
    }
    if let Err(e) = fs::rename(log_path, &previous)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e);
    }
    Ok(())
}

struct BackgroundLogger {
    sender: mpsc::Sender<LogMessage>,
    thread_handle: Option<JoinHandle<()>>,
}

impl BackgroundLogger {
    fn new(path: PathBuf) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel();

        let thread_handle = thread::Builder::new()
            .name("logger-thread".to_string())
            .spawn(move || {
                let mut logger = match Logger::new(&path) {
                    Ok(logger) => logger,
                    Err(e) => {
                        eprintln!("Failed to initialize logger: {}", e);
                        return;
                    }
                };

                loop {
                    match receiver.recv() {
                        Ok(LogMessage::Content(data)) => {
                            if let Err(e) = logger.write_all(&data) {
                                eprintln!("Logging error: {}", e);
                            }
                        }
                        Ok(LogMessage::Flush) => {
                            if let Err(e) = logger.flush() {
                                eprintln!("Error flushing logs: {}", e);
                            }
                        }
                        Ok(LogMessage::Shutdown) | Err(_) => break,
                    }
                }
            })?;

        Ok(BackgroundLogger {
            sender,
            thread_handle: Some(thread_handle),
        })
    }
}

impl Write for BackgroundLogger {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len();
        self.sender
            .send(LogMessage::Content(buf.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Logger thread disconnected"))?;
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sender
            .send(LogMessage::Flush)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Logger thread disconnected"))?;
        Ok(())
    }
}

impl Drop for BackgroundLogger {
    fn drop(&mut self) {
        let _ = self.sender.send(LogMessage::Shutdown);
        if let Some(handle) = self.thread_handle.take()
            && let Err(err) = handle.join()
        {
            eprintln!("Background logger thread panicked: {err:?}");
        }
    }
}

struct Logger {
    file: fs::File,
    current_size: u64,
}

impl Logger {
    fn new(path: &PathBuf) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let size = file.metadata()?.len();

        Ok(Logger {
            file,
            current_size: size,
        })
    }
}

impl Write for Logger {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let buf_len = buf.len() as u64;

        if self.current_size + buf_len > MAX_LOG_FILE_SIZE {
            let timestamp = chrono::Local::now().format("%H:%M:%S%.3f");
            let error_msg = format!(
                "\n{}:FATAL -- Log file size would exceed the maximum allowed size of {} bytes\n",
                timestamp, MAX_LOG_FILE_SIZE
            );

            eprintln!("{error_msg}");

            let _ = self.file.write_all(error_msg.as_bytes());
            let _ = self.file.flush();

            process::abort();
        }

        let bytes = self.file.write(buf)?;
        self.current_size += bytes as u64;

        Ok(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

pub fn install_panic_hook() {
    static PANIC_HOOK: Once = Once::new();

    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();

        std::panic::set_hook(Box::new(move |panic_info| {
            let report = format_panic_report(panic_info);

            log::error!(target: "panic", "{report}");
            log::logger().flush();

            if let Err(err) = append_stderr_log_line(&report) {
                eprintln!("Failed to persist panic report: {err}");
            }

            previous(panic_info);
        }));
    });
}

pub fn report_stderr(message: &str) {
    if let Err(err) = append_stderr_log_line(message) {
        eprintln!("Failed to persist std log entry: {err}");
    }

    eprintln!("{message}");
}

fn format_panic_report(info: &PanicHookInfo<'_>) -> String {
    let current_thread = thread::current();
    let thread_name = current_thread.name().unwrap_or("unnamed");
    let location = info
        .location()
        .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
        .unwrap_or_else(|| "unknown location".to_string());

    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_string());

    let backtrace = Backtrace::force_capture();

    format!("panic in thread '{thread_name}' at {location}: {payload}\nBacktrace:\n{backtrace}")
}

fn append_stderr_log_line(message: &str) -> io::Result<()> {
    let log_path = data::log::path().map_err(|err| io::Error::other(err.to_string()))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;

    writeln!(
        file,
        "{}:FATAL -- {message}",
        chrono::Local::now().format("%H:%M:%S%.3f"),
    )?;

    file.flush()
}
