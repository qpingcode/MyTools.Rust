//! The MyTools log: one JSON object per line in `logs/mytools.log`, next to `crash.log`.
//!
//! Only the host writes the file ([`init_file`]), so rotation never races between
//! processes. Everywhere else, including plugins, records go to stderr as JSON lines; the
//! host reads each plugin's stderr and writes them with the plugin as their source. Plain
//! text a plugin prints to stderr is kept too, as a warning.
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const FILE: &str = "mytools.log";
/// A file larger than this moves to `mytools.log.1` (and older ones up by one).
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// How many files are kept, counting the current one.
pub const KEEP: usize = 10;
/// The source of records written by MyTools itself.
pub const HOST: &str = "mytools";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Record {
    /// Local time, e.g. `2026-11-11 11:06:06.309`; set by the writer.
    #[serde(default)]
    pub time: String,
    pub level: Level,
    /// `mytools` or a plugin id; set by the host for plugin records.
    #[serde(default)]
    pub source: String,
    /// A stable code such as `plugin.ready`, for filtering; empty for free text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub event: String,
    pub message: String,
    /// More lines, such as a crash's backtrace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Record {
    pub fn new(level: Level, event: &str, message: impl Into<String>) -> Self {
        Self {
            time: String::new(),
            level,
            source: String::new(),
            event: event.into(),
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// A line a plugin printed to stderr: a record if it is one, else a warning.
    pub fn from_stderr(line: &str) -> Self {
        serde_json::from_str::<Self>(line)
            .unwrap_or_else(|_| Self::new(Level::Warn, "stderr", line.trim_end()))
    }
}

struct FileSink {
    directory: PathBuf,
    minimum: Level,
}

static SINK: OnceLock<Mutex<FileSink>> = OnceLock::new();

/// Makes this process (the host) write records to `directory/mytools.log`, dropping those
/// below `minimum`. Call once at startup.
pub fn init_file(directory: PathBuf, minimum: Level) {
    let _ = SINK.set(Mutex::new(FileSink { directory, minimum }));
}

/// Writes a record from this process; its source is the host when the file is ours.
pub fn write(mut record: Record) {
    if SINK.get().is_some() && record.source.is_empty() {
        record.source = HOST.into();
    }
    emit(record);
}

/// Writes a record on behalf of a plugin (host only).
pub fn write_from(source: &str, mut record: Record) {
    record.source = source.into();
    emit(record);
}

fn emit(mut record: Record) {
    record.time = crate::crash::timestamp();
    match SINK.get() {
        Some(sink) => {
            let sink = sink.lock().unwrap_or_else(|error| error.into_inner());
            if record.level >= sink.minimum {
                // Best effort: logging must never fail the work it describes.
                let _ = append(&sink.directory, &record);
            }
        }
        None => {
            if let Ok(line) = serde_json::to_string(&record) {
                let _ = writeln!(io::stderr(), "{line}");
            }
        }
    }
}

pub fn debug(event: &str, message: impl Into<String>) {
    write(Record::new(Level::Debug, event, message));
}

pub fn info(event: &str, message: impl Into<String>) {
    write(Record::new(Level::Info, event, message));
}

pub fn warn(event: &str, message: impl Into<String>) {
    write(Record::new(Level::Warn, event, message));
}

pub fn error(event: &str, message: impl Into<String>) {
    write(Record::new(Level::Error, event, message));
}

/// `mytools.log`, then `mytools.log.1` for the newest rotated file, and so on.
pub fn path(directory: &Path, index: usize) -> PathBuf {
    if index == 0 {
        directory.join(FILE)
    } else {
        directory.join(format!("{FILE}.{index}"))
    }
}

fn append(directory: &Path, record: &Record) -> io::Result<()> {
    let mut line = serde_json::to_string(record).map_err(io::Error::other)?;
    line.push('\n');
    std::fs::create_dir_all(directory)?;
    let current = path(directory, 0);
    if std::fs::metadata(&current).is_ok_and(|meta| meta.len() + line.len() as u64 > MAX_BYTES) {
        rotate(directory);
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(current)?
        .write_all(line.as_bytes())
}

fn rotate(directory: &Path) {
    let _ = std::fs::remove_file(path(directory, KEEP - 1));
    for index in (0..KEEP - 1).rev() {
        let _ = std::fs::rename(path(directory, index), path(directory, index + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_plain_stderr_is_a_warning() {
        let record =
            Record::new(Level::Error, "plugin.start_failed", "初始化失败").with_detail("backtrace");
        let line = serde_json::to_string(&record).unwrap();
        assert_eq!(Record::from_stderr(&line), record);
        let plain = Record::from_stderr("thread 'main' panicked\n");
        assert_eq!(plain.level, Level::Warn);
        assert_eq!(plain.event, "stderr");
        assert_eq!(plain.message, "thread 'main' panicked");
        assert!(Level::Warn > Level::Info);
        assert_eq!(Level::parse("WARNING"), Some(Level::Warn));
        assert_eq!(Level::parse("verbose"), None);
    }

    #[test]
    fn rotation_keeps_ten_files() {
        let directory = std::env::temp_dir().join(format!("mytools-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let record = Record::new(Level::Info, "test", "x".repeat(100));
        append(&directory, &record).unwrap();
        for _ in 0..KEEP + 3 {
            std::fs::write(path(&directory, 0), vec![b'x'; MAX_BYTES as usize]).unwrap();
            append(&directory, &record).unwrap();
        }
        assert!(path(&directory, KEEP - 1).exists());
        assert!(!path(&directory, KEEP).exists());
        let current = std::fs::read_to_string(path(&directory, 0)).unwrap();
        assert_eq!(current.lines().count(), 1);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
