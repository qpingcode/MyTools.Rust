//! What each plugin is doing right now, as the host publishes it for the settings window,
//! and the requests the settings window sends back (such as restarting a plugin).
use crate::plugin_settings::Startup;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Started on demand and not needed yet.
    Idle,
    /// The process is running its init.
    Starting,
    /// Answering searches.
    Ready,
    /// Stopped after a failed or timed-out start, or repeated crashes, until restarted.
    Failed,
    /// Turned off in the settings.
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginStatus {
    pub id: String,
    pub state: State,
    /// Why it failed or is disabled, or what a starting plugin reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The policy in effect; `None` for plugins without a process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<Startup>,
}

/// The host rewrites this whenever a plugin changes state.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StatusFile {
    /// The host process; a file left by a host that has exited is stale.
    pub pid: u32,
    pub plugins: Vec<PluginStatus>,
}

impl StatusFile {
    pub fn get(&self, id: &str) -> Option<&PluginStatus> {
        self.plugins.iter().find(|status| status.id == id)
    }
}

/// Plugins the settings window asks the host to restart; the host deletes the file once
/// it has read it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Requests {
    #[serde(default)]
    pub restart: Vec<String>,
}

/// `plugin-status.json`, next to the plugin settings file.
pub fn path(settings: &Path) -> PathBuf {
    settings.with_file_name("plugin-status.json")
}

/// `plugin-requests.json`, next to the plugin settings file.
pub fn requests_path(settings: &Path) -> PathBuf {
    settings.with_file_name("plugin-requests.json")
}

pub fn load(path: &Path) -> io::Result<StatusFile> {
    serde_json::from_slice(&std::fs::read(path)?).map_err(io::Error::other)
}

/// Writes through a temporary file so readers never see a half-written file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Adds a restart request, keeping requests the host has not read yet.
pub fn request_restart(settings: &Path, id: &str) -> io::Result<()> {
    let path = requests_path(settings);
    let mut requests = take_requests_at(&path).unwrap_or_default();
    if !requests.restart.iter().any(|known| known == id) {
        requests.restart.push(id.into());
    }
    save(&path, &requests)
}

/// Reads and removes pending requests; `None` when there are none.
pub fn take_requests(settings: &Path) -> Option<Requests> {
    take_requests_at(&requests_path(settings))
}

fn take_requests_at(path: &Path) -> Option<Requests> {
    let bytes = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_and_requests_are_taken_once() {
        let directory = std::env::temp_dir().join(format!("plugin-status-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let settings = directory.join("plugin-settings.json");
        let status = StatusFile {
            pid: 7,
            plugins: vec![PluginStatus {
                id: "everything".into(),
                state: State::Failed,
                detail: Some("timed out".into()),
                startup: Some(Startup::Lazy),
            }],
        };
        save(&path(&settings), &status).unwrap();
        assert_eq!(load(&path(&settings)).unwrap(), status);
        assert_eq!(
            load(&path(&settings))
                .unwrap()
                .get("everything")
                .unwrap()
                .state,
            State::Failed
        );

        assert!(take_requests(&settings).is_none());
        request_restart(&settings, "everything").unwrap();
        request_restart(&settings, "everything").unwrap();
        request_restart(&settings, "clipboard").unwrap();
        assert_eq!(
            take_requests(&settings).unwrap().restart,
            ["everything", "clipboard"]
        );
        assert!(take_requests(&settings).is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
