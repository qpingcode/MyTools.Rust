//! Where MyTools keeps each user's files. The install folder holds only the program; the
//! user's settings (`config.json`, `plugin-settings.json`), runtime state
//! (`window-position.json`, `plugin-status.json`), the search history, the logs and each
//! plugin's own folder (its `config.json` and the data it keeps) live under one root:
//!
//! - Windows: `%LOCALAPPDATA%\MyTools`
//! - macOS: `~/Library/Application Support/MyTools`
//! - elsewhere: `$XDG_DATA_HOME/MyTools`, by default `~/.local/share/MyTools`
//!
//! `MYTOOLS_DATA_DIR` replaces the root; plugin processes inherit it from the host.
use std::{ffi::OsString, path::PathBuf};

pub const ENV: &str = "MYTOOLS_DATA_DIR";

/// The root; the executable's folder when the platform gives no location.
pub fn root() -> PathBuf {
    resolve(|name| std::env::var_os(name)).unwrap_or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from))
            .unwrap_or_default()
    })
}

/// A plugin's own folder, named by its id: its configuration files and its data.
pub fn plugin(id: &str) -> PathBuf {
    root().join(id)
}

/// `mytools.log` and `crash.log`.
pub fn logs() -> PathBuf {
    root().join("logs")
}

fn resolve(variable: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let variable = |name: &str| variable(name).filter(|value| !value.is_empty());
    if let Some(root) = variable(ENV) {
        return Some(root.into());
    }
    #[cfg(windows)]
    return variable("LOCALAPPDATA").map(|local| PathBuf::from(local).join("MyTools"));
    #[cfg(target_os = "macos")]
    return variable("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/MyTools"));
    #[cfg(not(any(windows, target_os = "macos")))]
    variable("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| variable("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|data| data.join("MyTools"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(pairs: &[(&str, &str)]) -> Option<PathBuf> {
        resolve(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn the_override_wins_and_empty_values_are_ignored() {
        let all = [
            (ENV, "/data"),
            ("LOCALAPPDATA", "/local"),
            ("HOME", "/home/me"),
            ("XDG_DATA_HOME", "/xdg"),
        ];
        assert_eq!(with(&all), Some(PathBuf::from("/data")));
        let platform = with(&all[1..]).unwrap();
        assert!(platform.ends_with("MyTools"), "{platform:?}");
        assert_eq!(
            with(&[
                (ENV, ""),
                ("LOCALAPPDATA", ""),
                ("HOME", ""),
                ("XDG_DATA_HOME", "")
            ]),
            None
        );
        assert_eq!(with(&[]), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_local_app_data() {
        assert_eq!(
            with(&[("LOCALAPPDATA", r"C:\Users\me\AppData\Local")]),
            Some(PathBuf::from(r"C:\Users\me\AppData\Local\MyTools"))
        );
    }
}
