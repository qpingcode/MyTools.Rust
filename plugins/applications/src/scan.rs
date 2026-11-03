use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub fn roots() -> Vec<PathBuf> {
    if let Some(paths) = std::env::var_os("MYTOOLS_APPLICATION_DIRS") {
        return std::env::split_paths(&paths)
            .filter(|path| path.is_absolute())
            .collect();
    }
    default_roots()
}

#[cfg(windows)]
fn default_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for variable in ["APPDATA", "ProgramData"] {
        if let Some(base) = std::env::var_os(variable) {
            roots.push(PathBuf::from(base).join("Microsoft/Windows/Start Menu"));
        }
    }
    for variable in ["USERPROFILE", "PUBLIC"] {
        if let Some(base) = std::env::var_os(variable) {
            roots.push(PathBuf::from(base).join("Desktop"));
        }
    }
    roots
}

#[cfg(target_os = "macos")]
fn default_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
}

#[cfg(not(any(windows, target_os = "macos")))]
fn default_roots() -> Vec<PathBuf> {
    Vec::new()
}

pub fn applications(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut pending = Vec::new();
    let mut visited = HashSet::new();
    for root in roots {
        if let Ok(root) = root.canonicalize() {
            pending.push(root);
        }
    }
    let mut applications = Vec::new();
    while let Some(directory) = pending.pop() {
        if !visited.insert(path_key(&directory)) {
            continue;
        }
        if cfg!(target_os = "macos") && extension(&directory, "app") && directory.is_dir() {
            applications.push(directory);
            continue;
        }
        let Ok(children) = fs::read_dir(directory) else {
            continue;
        };
        for child in children.flatten() {
            let Ok(kind) = child.file_type() else {
                continue;
            };
            // Never follow directory symlinks/junctions into loops or other trees.
            if kind.is_symlink() {
                continue;
            }
            let path = child.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() && (extension(&path, "lnk") || extension(&path, "url")) {
                applications.push(path);
            }
        }
    }
    applications.sort();
    applications.dedup_by(|left, right| path_key(left) == path_key(right));
    applications
}

fn extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn path_key(path: &Path) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        path.as_os_str().to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_nested_shortcuts_once_and_ignores_other_files_and_missing_roots() {
        let root = std::env::temp_dir().join(format!("mytools-app-scan-{}", std::process::id()));
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        for path in [
            root.join("Editor.LNK"),
            nested.join("Website.url"),
            nested.join("notes.txt"),
        ] {
            fs::write(path, b"fixture").unwrap();
        }
        let results = applications(&[root.clone(), nested.clone(), root.join("missing")]);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|path| path.is_absolute()));
        assert!(
            results
                .iter()
                .any(|path| path.file_name().unwrap() == "Editor.LNK")
        );
        assert!(
            results
                .iter()
                .any(|path| path.file_name().unwrap() == "Website.url")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn indexes_app_bundles_without_scanning_their_contents() {
        let root = std::env::temp_dir().join(format!("mytools-app-bundle-{}", std::process::id()));
        let bundle = root.join("Editor.app");
        fs::create_dir_all(bundle.join("Contents/Nested.app")).unwrap();
        let results = applications(&[root.clone(), bundle.clone()]);
        assert_eq!(results, vec![bundle.canonicalize().unwrap()]);
        fs::remove_dir_all(root).unwrap();
    }
}
