//! Reads and rewrites Chrome's state files in its user data directory.
use crate::policy;
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

/// Applies the Gemini policy to `Local State` and every user profile's `Preferences`.
/// Chrome must be closed: it rewrites both files from memory when it exits.
pub fn repair(user_data: &Path) -> Result<(), String> {
    patch_file(&user_data.join("Local State"), policy::patch_local_state)?;
    for profile in profiles(user_data) {
        patch_file(&profile.join("Preferences"), policy::patch_preferences)?;
    }
    Ok(())
}

/// The profile Chrome opened last, so the restarted Chrome shows the same one.
pub fn last_used(user_data: &Path) -> String {
    read_json(&user_data.join("Local State"))
        .ok()
        .flatten()
        .and_then(|state| state["profile"]["last_used"].as_str().map(str::to_owned))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Default".into())
}

fn profiles(user_data: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(user_data) else {
        return vec![];
    };
    let mut profiles: Vec<_> = entries
        .flatten()
        .filter(|entry| is_user_profile(&entry.file_name().to_string_lossy()))
        .map(|entry| entry.path())
        .collect();
    profiles.sort();
    profiles
}

/// `Default` and `Profile N`; not `System Profile`, `Guest Profile` and the like.
fn is_user_profile(name: &str) -> bool {
    name == "Default"
        || name
            .strip_prefix("Profile ")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

fn patch_file(path: &Path, patch: fn(&mut Value)) -> Result<(), String> {
    let failed = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    // A missing file is created by Chrome on its first run; nothing to fix yet.
    let Some(original) = read_json(path).map_err(|error| failed(&error))? else {
        return Ok(());
    };
    let mut patched = original.clone();
    patch(&mut patched);
    if patched == original {
        return Ok(());
    }
    backup(path).map_err(|error| failed(&error))?;
    write(path, &patched).map_err(|error| failed(&error))
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Keeps the file as it was before MyTools first changed it.
fn backup(path: &Path) -> io::Result<()> {
    let backup = sibling(path, "mytools-backup");
    if backup.exists() {
        return Ok(());
    }
    fs::copy(path, backup).map(drop)
}

/// Writes through a temporary file so Chrome never reads half a file. A just-closed Chrome
/// or a virus scanner can hold the file briefly, so the rename is retried.
fn write(path: &Path, value: &Value) -> io::Result<()> {
    let temporary = sibling(path, "mytools.tmp");
    fs::write(&temporary, value.to_string())?;
    let mut result = fs::rename(&temporary, path);
    for attempt in 0..5 {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100 << attempt));
        result = fs::rename(&temporary, path);
    }
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{suffix}"));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_data(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("mytools-gemini-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for profile in ["Default", "Profile 2", "System Profile"] {
            fs::create_dir_all(root.join(profile)).unwrap();
            fs::write(root.join(profile).join("Preferences"), "{}").unwrap();
        }
        root
    }

    #[test]
    fn repairs_user_profiles_and_keeps_a_backup() {
        let root = user_data("repair");
        fs::write(
            root.join("Local State"),
            json!({"profile": {"last_used": "Profile 2"}}).to_string(),
        )
        .unwrap();
        repair(&root).unwrap();
        let preferences = read_json(&root.join("Profile 2/Preferences"))
            .unwrap()
            .unwrap();
        assert_eq!(preferences["glic"]["completed_fre"], 1);
        assert_eq!(
            fs::read_to_string(root.join("Profile 2/Preferences.mytools-backup")).unwrap(),
            "{}"
        );
        // Chrome's own profiles are not touched.
        assert_eq!(
            fs::read_to_string(root.join("System Profile/Preferences")).unwrap(),
            "{}"
        );
        assert_eq!(last_used(&root), "Profile 2");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_fresh_chrome_without_local_state_starts_its_default_profile() {
        let root = user_data("fresh");
        repair(&root).unwrap();
        assert!(!root.join("Local State").exists());
        assert_eq!(last_used(&root), "Default");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_broken_file_is_reported_and_left_alone() {
        let root = user_data("broken");
        fs::write(root.join("Local State"), "{not json").unwrap();
        assert!(repair(&root).unwrap_err().contains("Local State"));
        assert_eq!(
            fs::read_to_string(root.join("Local State")).unwrap(),
            "{not json"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
