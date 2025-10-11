//! LSP logging infrastructure
//!
//! Logs LSP messages to a file instead of stderr/stdout to avoid cluttering the terminal.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref LSP_LOG_FILE: Mutex<Option<File>> = Mutex::new(None);
}

/// Initialize LSP logging to a file
pub fn init_lsp_logging() -> std::io::Result<()> {
    let log_path = get_log_path();

    // Create parent directory if needed
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let mut log_file = LSP_LOG_FILE.lock().unwrap();
    *log_file = Some(file);

    Ok(())
}

/// Get the path to the LSP log file
fn get_log_path() -> PathBuf {
    if let Ok(cache_dir) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(cache_dir).join("ovim").join("lsp.log")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("ovim").join("lsp.log")
    } else {
        PathBuf::from("/tmp").join("ovim-lsp.log")
    }
}

/// Log levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// Log a message to the LSP log file
pub fn log_message(level: LogLevel, context: &str, message: &str) {
    // Only log debug messages if OVIM_LSP_DEBUG is set
    if level == LogLevel::Debug && std::env::var("OVIM_LSP_DEBUG").is_err() {
        return;
    }

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let log_line = format!("[{}] [{}] [{}] {}\n", timestamp, level.as_str(), context, message);

    if let Ok(mut log_file) = LSP_LOG_FILE.lock() {
        if let Some(ref mut file) = *log_file {
            let _ = file.write_all(log_line.as_bytes());
            let _ = file.flush();
        }
    }
}

/// Convenience macros for logging
#[macro_export]
macro_rules! lsp_debug {
    ($context:expr, $($arg:tt)*) => {
        $crate::lsp::logger::log_message(
            $crate::lsp::logger::LogLevel::Debug,
            $context,
            &format!($($arg)*)
        )
    };
}

#[macro_export]
macro_rules! lsp_info {
    ($context:expr, $($arg:tt)*) => {
        $crate::lsp::logger::log_message(
            $crate::lsp::logger::LogLevel::Info,
            $context,
            &format!($($arg)*)
        )
    };
}

#[macro_export]
macro_rules! lsp_warn {
    ($context:expr, $($arg:tt)*) => {
        $crate::lsp::logger::log_message(
            $crate::lsp::logger::LogLevel::Warning,
            $context,
            &format!($($arg)*)
        )
    };
}

#[macro_export]
macro_rules! lsp_error {
    ($context:expr, $($arg:tt)*) => {
        $crate::lsp::logger::log_message(
            $crate::lsp::logger::LogLevel::Error,
            $context,
            &format!($($arg)*)
        )
    };
}
