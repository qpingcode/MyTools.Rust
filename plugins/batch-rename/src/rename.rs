use crate::rules::Rules;
use mytools_plugin_api::{i18n::Catalog, paths};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub from: PathBuf,
    pub to: PathBuf,
}

pub fn plan(
    selected: &[PathBuf],
    rules: &Rules<'_>,
    catalog: &Catalog,
) -> Result<Vec<Change>, String> {
    let mut sources = Vec::with_capacity(selected.len());
    let mut seen = HashSet::new();
    for path in selected {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|_| path.is_absolute())
            .ok_or_else(|| catalog.format("name.invalid", &[("name", &paths::display(path))]))?;
        let parent = path
            .parent()
            .expect("an absolute path with a file name has a parent");
        let source = fs::canonicalize(parent)
            .map_err(|error| unavailable(path, &error, catalog))?
            .join(name);
        if seen.insert(source.clone()) {
            let metadata = fs::symlink_metadata(&source)
                .map_err(|error| unavailable(path, &error, catalog))?;
            sources.push((source, metadata.is_dir()));
        }
    }
    let mut changes = Vec::new();
    for (index, (source, directory)) in sources.iter().enumerate() {
        let original = source.file_name().and_then(|name| name.to_str()).unwrap();
        let name = rules.name(original, *directory, index, catalog)?;
        if name == original {
            continue;
        }
        if *directory
            && sources
                .iter()
                .any(|(other, _)| other != source && other.starts_with(source))
        {
            return Err(catalog.format("selection.nested", &[("path", &paths::display(source))]));
        }
        changes.push(Change {
            from: source.clone(),
            to: source.with_file_name(name),
        });
    }
    validate_targets(&changes, catalog)?;
    Ok(changes)
}

fn path_key(path: &Path) -> String {
    let text = paths::display(path);
    if cfg!(any(windows, target_os = "macos")) {
        text.to_lowercase()
    } else {
        text
    }
}

fn validate_targets(changes: &[Change], catalog: &Catalog) -> Result<(), String> {
    let mut destinations = HashSet::new();
    for change in changes {
        if !destinations.insert(path_key(&change.to)) {
            return Err(conflict(&change.to, catalog));
        }
        match fs::symlink_metadata(&change.to) {
            Ok(_) if same_name_ignoring_case(change) => {}
            Ok(_) => return Err(conflict(&change.to, catalog)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(unavailable(&change.to, &error, catalog)),
        }
    }
    Ok(())
}

fn same_name_ignoring_case(change: &Change) -> bool {
    path_key(&change.from) == path_key(&change.to)
        && fs::canonicalize(&change.from)
            .ok()
            .zip(fs::canonicalize(&change.to).ok())
            .is_some_and(|(source, destination)| source.as_os_str() == destination.as_os_str())
}

fn unavailable(path: &Path, error: &io::Error, catalog: &Catalog) -> String {
    catalog.format(
        "name.unavailable",
        &[
            ("path", &paths::display(path)),
            ("error", &error.to_string()),
        ],
    )
}

fn conflict(path: &Path, catalog: &Catalog) -> String {
    catalog.format("name.conflict", &[("path", &paths::display(path))])
}

pub fn apply(changes: &[Change], catalog: &Catalog) -> Result<(), String> {
    // Preflight again immediately before applying. The native move also rejects races
    // where another process creates a destination after this check.
    validate_targets(changes, catalog)?;
    for change in changes {
        fs::symlink_metadata(&change.from)
            .map_err(|error| unavailable(&change.from, &error, catalog))?;
    }
    for (index, change) in changes.iter().enumerate() {
        if let Err(error) = move_exclusive(&change.from, &change.to) {
            let message = catalog.format(
                "rename.failed",
                &[
                    ("path", &paths::display(&change.from)),
                    ("error", &error.to_string()),
                ],
            );
            let stranded: Vec<_> = changes[..index]
                .iter()
                .rev()
                .filter_map(|previous| {
                    move_exclusive(&previous.to, &previous.from)
                        .err()
                        .map(|_| paths::display(&previous.to))
                })
                .collect();
            return Err(if stranded.is_empty() {
                message
            } else {
                format!(
                    "{message}\n\n{}",
                    catalog.format("rename.rollback_failed", &[("paths", &stranded.join("\n"))])
                )
            });
        }
    }
    Ok(())
}

#[cfg(windows)]
fn move_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    if unsafe { MoveFileExW(wide(from).as_ptr(), wide(to).as_ptr(), 0) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn move_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn move_exclusive(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
