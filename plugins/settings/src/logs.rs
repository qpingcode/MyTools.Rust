//! The log page's data: the newest records of `logs/mytools.log` and its rotated files,
//! followed as MyTools appends to it, and the filters the page applies.
use mytools_plugin_api::log::{self, Level, Record};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// The page keeps at most this many records, the newest ones.
pub const MAX_RECORDS: usize = 2000;

/// A record as read, numbered so the page can remember which ones are expanded.
pub struct Entry {
    pub id: u64,
    pub record: Record,
}

pub struct Logs {
    directory: PathBuf,
    /// Oldest first.
    pub entries: Vec<Entry>,
    next_id: u64,
    /// How far `mytools.log` has been read, and an incomplete last line kept for later.
    offset: u64,
    partial: String,
}

impl Logs {
    pub fn open(directory: PathBuf) -> Self {
        let mut logs = Self {
            directory,
            entries: Vec::new(),
            next_id: 0,
            offset: 0,
            partial: String::new(),
        };
        logs.reload();
        logs
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Reads the newest records again from the current and rotated files.
    pub fn reload(&mut self) {
        self.entries.clear();
        self.partial.clear();
        let mut lines: Vec<String> = Vec::new();
        for index in 0..log::KEEP {
            if lines.len() >= MAX_RECORDS {
                break;
            }
            let path = log::path(&self.directory, index);
            let Ok(bytes) = std::fs::read(&path) else {
                if index == 0 {
                    self.offset = 0;
                }
                continue;
            };
            if index == 0 {
                self.offset = bytes.len() as u64;
            }
            let text = String::from_utf8_lossy(&bytes);
            // Older files go in front of what was read so far.
            let mut older: Vec<String> = text.lines().map(str::to_owned).collect();
            older.append(&mut lines);
            lines = older;
        }
        let skip = lines.len().saturating_sub(MAX_RECORDS);
        for line in &lines[skip..] {
            self.push(line);
        }
    }

    /// Picks up lines appended since the last call; `true` when anything changed. A file
    /// that shrank was rotated, so everything is read again.
    pub fn refresh(&mut self) -> bool {
        let current = log::path(&self.directory, 0);
        let length = std::fs::metadata(&current).map_or(0, |meta| meta.len());
        if length < self.offset {
            self.reload();
            return true;
        }
        if length == self.offset {
            return false;
        }
        let Ok(mut file) = std::fs::File::open(&current) else {
            return false;
        };
        let mut added = Vec::new();
        if file.seek(SeekFrom::Start(self.offset)).is_err() || file.read_to_end(&mut added).is_err()
        {
            return false;
        }
        self.offset += added.len() as u64;
        let mut text = std::mem::take(&mut self.partial);
        text.push_str(&String::from_utf8_lossy(&added));
        let complete = text.ends_with('\n');
        let mut lines: Vec<&str> = text.lines().collect();
        if !complete {
            self.partial = lines.pop().unwrap_or_default().to_owned();
        }
        let before = self.next_id;
        for line in lines {
            self.push(line);
        }
        let excess = self.entries.len().saturating_sub(MAX_RECORDS);
        self.entries.drain(..excess);
        self.next_id != before
    }

    fn push(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        // A line that is not a record (e.g. written by hand) still shows.
        let record = serde_json::from_str(line)
            .unwrap_or_else(|_| Record::new(Level::Info, "", line.trim_end()));
        self.entries.push(Entry {
            id: self.next_id,
            record,
        });
        self.next_id += 1;
    }
}

/// What the page shows: one source or all, a least severe level and words to find.
pub struct Filter {
    pub source: Option<String>,
    pub level: Level,
    pub text: String,
}

impl Filter {
    pub fn matches(&self, record: &Record) -> bool {
        if record.level < self.level {
            return false;
        }
        if self
            .source
            .as_ref()
            .is_some_and(|source| *source != record.source)
        {
            return false;
        }
        let text = self.text.trim().to_lowercase();
        text.is_empty()
            || [
                record.message.as_str(),
                record.event.as_str(),
                record.source.as_str(),
                record.detail.as_deref().unwrap_or_default(),
            ]
            .iter()
            .any(|field| field.to_lowercase().contains(&text))
    }
}

/// One record as plain text, for copying.
pub fn plain(record: &Record) -> String {
    let level = match record.level {
        Level::Debug => "DEBUG",
        Level::Info => "INFO",
        Level::Warn => "WARN",
        Level::Error => "ERROR",
    };
    let mut text = format!(
        "{} {level} [{}] {} {}",
        record.time, record.source, record.event, record.message
    );
    if let Some(detail) = &record.detail {
        text.push('\n');
        text.push_str(detail);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(level: Level, source: &str, message: &str) -> String {
        let mut record = Record::new(level, "test", message);
        record.source = source.into();
        record.time = "2026-11-11 11:00:00.000".into();
        serde_json::to_string(&record).unwrap() + "\n"
    }

    #[test]
    fn reads_rotated_files_follows_appends_and_reloads_after_rotation() {
        let directory = std::env::temp_dir().join(format!("settings-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            log::path(&directory, 1),
            line(Level::Info, "mytools", "older"),
        )
        .unwrap();
        std::fs::write(
            log::path(&directory, 0),
            line(Level::Warn, "crash-test", "newer"),
        )
        .unwrap();
        let mut logs = Logs::open(directory.clone());
        let messages: Vec<_> = logs
            .entries
            .iter()
            .map(|e| e.record.message.as_str())
            .collect();
        assert_eq!(messages, ["older", "newer"]);
        assert!(!logs.refresh());

        // A line written in two parts is taken once it is complete.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(log::path(&directory, 0))
            .unwrap();
        let third = line(Level::Error, "crash-test", "third");
        let (head, tail) = third.split_at(10);
        file.write_all(head.as_bytes()).unwrap();
        file.flush().unwrap();
        assert!(!logs.refresh());
        file.write_all(tail.as_bytes()).unwrap();
        file.flush().unwrap();
        assert!(logs.refresh());
        assert_eq!(logs.entries.last().unwrap().record.message, "third");

        // Rotation: the current file starts over, smaller than what was read.
        std::fs::write(
            log::path(&directory, 0),
            line(Level::Info, "mytools", "fresh"),
        )
        .unwrap();
        assert!(logs.refresh());
        assert_eq!(logs.entries.last().unwrap().record.message, "fresh");
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn filters_by_source_level_and_text() {
        let mut record = Record::new(Level::Warn, "plugin.crashed", "exited with code 101");
        record.source = "crash-test".into();
        let all = Filter {
            source: None,
            level: Level::Info,
            text: String::new(),
        };
        assert!(all.matches(&record));
        assert!(
            !Filter {
                level: Level::Error,
                ..all
            }
            .matches(&record)
        );
        let all = Filter {
            source: Some("mytools".into()),
            level: Level::Debug,
            text: String::new(),
        };
        assert!(!all.matches(&record));
        let found = Filter {
            source: Some("crash-test".into()),
            level: Level::Debug,
            text: "CODE 101".into(),
        };
        assert!(found.matches(&record));
        assert!(plain(&record).contains("WARN [crash-test] plugin.crashed"));
    }
}
