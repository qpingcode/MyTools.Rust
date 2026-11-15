use super::{
    Change, LIMIT, Plan, TEXT_LIMIT, name,
    platform::{FileId, file_id, open_entry},
};
use mytools_plugin_api::{i18n::Catalog, paths};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, Metadata},
    io::{self, Read},
    path::PathBuf,
};

#[derive(Clone, Default)]
pub struct Scope {
    selected: Vec<Entry>,
    directories: Vec<Entry>,
    files: Vec<Entry>,
    previous_proposal: Option<Value>,
}

#[derive(Clone)]
pub(super) struct Entry {
    pub(super) path: PathBuf,
    metadata: Metadata,
    id: FileId,
}

impl Entry {
    pub(super) fn new(path: PathBuf) -> io::Result<Self> {
        let file = open_entry(&path, false)?;
        let metadata = file.metadata()?;
        if is_link(&metadata) || !(metadata.is_file() || metadata.is_dir()) {
            return Err(io::Error::other("expected a regular file or directory"));
        }
        let id = file_id(&file)?;
        Ok(Self { path, metadata, id })
    }

    pub(super) fn check(&self) -> io::Result<()> {
        let current = Self::new(self.path.clone())?;
        if fs::canonicalize(&self.path)? != self.path
            || current.id != self.id
            || current.metadata.is_dir() != self.metadata.is_dir()
            || current.metadata.created().ok() != self.metadata.created().ok()
            || (self.metadata.is_file()
                && (current.metadata.modified().ok() != self.metadata.modified().ok()
                    || current.metadata.len() != self.metadata.len()))
        {
            return Err(io::Error::other("the file or directory changed"));
        }
        Ok(())
    }

    fn open_for_read(&self) -> io::Result<fs::File> {
        self.check()?;
        let file = open_entry(&self.path, true)?;
        if file_id(&file)? != self.id {
            return Err(io::Error::other("the file changed"));
        }
        Ok(file)
    }
}

#[cfg(windows)]
pub(super) fn is_link(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
pub(super) fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

impl Scope {
    pub fn new(paths: Vec<PathBuf>) -> Result<Self, String> {
        let mut scope = Self::default();
        for path in paths {
            let result = (|| {
                if !path.is_absolute() {
                    return Err(io::Error::other("expected an absolute path"));
                }
                Entry::new(path.clone())?;
                Entry::new(fs::canonicalize(&path)?)
            })();
            let entry = result.map_err(|error| format!("{}: {error}", paths::display(&path)))?;
            if scope.selected.iter().any(|item| item.path == entry.path) {
                continue;
            }
            if entry.metadata.is_dir() {
                scope.directories.push(entry.clone());
            } else {
                scope.files.push(entry.clone());
            }
            scope.selected.push(entry);
        }
        for directory in &scope.directories {
            let paths = fs::read_dir(&directory.path)
                .map_err(|error| error.to_string())?
                .take(LIMIT + 1)
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<io::Result<BTreeSet<_>>>()
                .map_err(|error| error.to_string())?;
            if paths.len() > LIMIT {
                return Err("too many entries in the attached directory".into());
            }
            for path in paths {
                if let Ok(file) = Entry::new(path)
                    && file.metadata.is_file()
                    && !scope.files.iter().any(|item| item.path == file.path)
                {
                    scope.files.push(file);
                }
            }
        }
        if scope.files.len() + scope.directories.len() > LIMIT {
            return Err("too many attached files".into());
        }
        Ok(scope)
    }

    pub fn update_after(&mut self, plan: &Plan) {
        for change in &plan.changes {
            match change {
                Change::Rename { from, to } => {
                    if let Some(entry) = self.selected.iter_mut().find(|entry| &entry.path == from)
                    {
                        entry.path = to.clone();
                    }
                }
                Change::Delete { path } => self.selected.retain(|entry| &entry.path != path),
                _ => {}
            }
        }
    }

    pub fn refresh(&mut self) {
        let paths = self
            .selected
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        *self = Self::new(paths).unwrap_or_default();
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    pub fn remember(&mut self, plan: Plan) {
        self.previous_proposal = Some(plan.proposal);
    }

    pub fn label(&self, catalog: &Catalog) -> String {
        catalog.format(
            "files.attachment",
            &[("count", &self.selected.len().to_string())],
        )
    }

    pub(super) fn entry(&self, id: usize, directory: bool) -> Result<&Entry, String> {
        let entries = if directory {
            &self.directories
        } else {
            &self.files
        };
        entries
            .get(id)
            .ok_or_else(|| "unknown attachment ID".into())
    }

    pub fn list(&self) -> Value {
        let items = |entries: &[Entry]| {
            entries
                .iter()
                .enumerate()
                .map(|(id, entry)| {
                    let directory = self
                        .directories
                        .iter()
                        .position(|directory| entry.path.parent() == Some(&directory.path));
                    json!({
                        "id": id,
                        "name": name(&entry.path),
                        "directory": directory,
                        "selected": self.selected.iter().any(|item| item.path == entry.path),
                    })
                })
                .collect::<Vec<_>>()
        };
        json!({
            "files": items(&self.files),
            "directories": items(&self.directories),
            "previous_proposal": self.previous_proposal,
        })
    }

    pub fn read(&self, id: usize) -> Result<String, String> {
        let entry = self.entry(id, false)?;
        let read = || {
            let mut bytes = Vec::new();
            entry
                .open_for_read()?
                .take((TEXT_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > TEXT_LIMIT {
                return Err(io::Error::other("text file exceeds 64 KiB"));
            }
            String::from_utf8(bytes).map_err(io::Error::other)
        };
        read().map_err(|error| error.to_string())
    }
}
