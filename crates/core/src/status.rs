//! The live state of every plugin: read by search routing and the status bar, published
//! to `plugin-status.json` for the settings window, and the source of problem notices.
use mytools_plugin_api::{
    plugin_settings::Startup,
    plugin_status::{self, PluginStatus, State, StatusFile},
};
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// A problem the user should see once, e.g. a plugin that timed out while starting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub message: String,
}

#[derive(Default)]
pub struct Board {
    plugins: Mutex<Vec<PluginStatus>>,
    revision: AtomicU64,
    notices: Mutex<Vec<Notice>>,
    /// Where the state is published; `None` keeps it in memory (tests).
    file: Option<PathBuf>,
}

impl Board {
    pub fn new(file: Option<PathBuf>) -> Self {
        Self {
            file,
            ..Default::default()
        }
    }

    pub fn register(
        &self,
        id: &str,
        state: State,
        detail: Option<String>,
        startup: Option<Startup>,
    ) {
        self.plugins.lock().unwrap().push(PluginStatus {
            id: id.into(),
            state,
            detail,
            startup,
        });
        self.changed();
    }

    pub fn set(&self, id: &str, state: State, detail: Option<String>) {
        let mut plugins = self.plugins.lock().unwrap();
        let Some(status) = plugins.iter_mut().find(|status| status.id == id) else {
            return;
        };
        if status.state == state && status.detail == detail {
            return;
        }
        status.state = state;
        status.detail = detail;
        drop(plugins);
        self.changed();
    }

    pub fn get(&self, id: &str) -> Option<(State, Option<String>)> {
        self.plugins
            .lock()
            .unwrap()
            .iter()
            .find(|status| status.id == id)
            .map(|status| (status.state, status.detail.clone()))
    }

    /// Bumped on every change, so a search waiting for a plugin can run again.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn notify(&self, notice: Notice) {
        self.notices.lock().unwrap().push(notice);
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut *self.notices.lock().unwrap())
    }

    /// Removes the published file, e.g. when MyTools exits.
    pub fn retract(&self) {
        if let Some(file) = &self.file {
            let _ = std::fs::remove_file(file);
        }
    }

    fn changed(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        if let Some(file) = &self.file {
            let snapshot = StatusFile {
                pid: std::process::id(),
                plugins: self.plugins.lock().unwrap().clone(),
            };
            let _ = plugin_status::save(file, &snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_bump_revision_and_publish_the_file() {
        let file = std::env::temp_dir().join(format!("board-{}.json", std::process::id()));
        let board = Board::new(Some(file.clone()));
        board.register("calculator", State::Starting, None, Some(Startup::Eager));
        let first = board.revision();
        board.set("calculator", State::Starting, None);
        assert_eq!(board.revision(), first, "unchanged state is not a change");
        board.set("calculator", State::Ready, None);
        assert!(board.revision() > first);
        assert_eq!(board.get("calculator").unwrap().0, State::Ready);
        let published = plugin_status::load(&file).unwrap();
        assert_eq!(published.get("calculator").unwrap().state, State::Ready);
        board.notify(Notice {
            title: "a".into(),
            message: "b".into(),
        });
        assert_eq!(board.take_notices().len(), 1);
        assert!(board.take_notices().is_empty());
        board.retract();
        assert!(!file.exists());
    }
}
