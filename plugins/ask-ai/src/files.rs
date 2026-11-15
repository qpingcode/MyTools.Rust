use mytools_plugin_api::{i18n::Catalog, paths};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

mod platform;
mod scope;
use platform::{move_exclusive, pin_directory};
use scope::Entry;
pub use scope::Scope;

const LIMIT: usize = 200;
const TEXT_LIMIT: usize = 64 * 1024;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Proposal {
    Rename {
        changes: Vec<Rename>,
    },
    Create {
        directory: usize,
        name: String,
        text: String,
    },
    Modify {
        file: usize,
        text: String,
    },
    Delete {
        file: usize,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    file: usize,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Change {
    Rename { from: PathBuf, to: PathBuf },
    Create { path: PathBuf, text: String },
    Modify { path: PathBuf, text: String },
    Delete { path: PathBuf },
}

pub struct Plan {
    sources: Vec<Entry>,
    directories: Vec<Entry>,
    changes: Vec<Change>,
    proposal: Value,
}

impl Plan {
    pub fn propose(scope: &Scope, arguments: &Value) -> Result<Self, String> {
        let proposal =
            serde_json::from_value(arguments.clone()).map_err(|error| error.to_string())?;
        Self::new(scope, proposal, arguments.clone())
    }

    fn new(scope: &Scope, proposal: Proposal, arguments: Value) -> Result<Self, String> {
        let mut plan = Self {
            sources: Vec::new(),
            directories: Vec::new(),
            changes: Vec::new(),
            proposal: arguments,
        };
        let mut file = |id| {
            let entry = scope.entry(id, false)?.clone();
            let path = entry.path.clone();
            plan.sources.push(entry);
            Ok::<_, String>(path)
        };
        match proposal {
            Proposal::Rename { changes } => {
                if changes.is_empty() || changes.len() > LIMIT {
                    return Err("invalid number of renames".into());
                }
                let mut seen = HashSet::new();
                for change in changes {
                    let from = file(change.file)?;
                    if !seen.insert(change.file) {
                        return Err("duplicate source file".into());
                    }
                    let to = from.with_file_name(valid_name(&change.name)?);
                    if name(&from) != change.name {
                        plan.changes.push(Change::Rename { from, to });
                    }
                }
            }
            Proposal::Create {
                directory,
                name,
                text,
            } => {
                let directory = scope.entry(directory, true)?;
                directory.check().map_err(|error| error.to_string())?;
                let root = &directory.path;
                plan.changes.push(Change::Create {
                    path: root.join(valid_name(&name)?),
                    text: limited(text)?,
                });
            }
            Proposal::Modify { file: id, text } => {
                plan.changes.push(Change::Modify {
                    path: file(id)?,
                    text: limited(text)?,
                });
            }
            Proposal::Delete { file: id } => {
                plan.changes.push(Change::Delete { path: file(id)? });
            }
        }
        if plan.changes.is_empty() {
            return Err("the plan contains no changes".into());
        }
        for change in &plan.changes {
            let root = change.path().parent().ok_or("missing parent directory")?;
            if !plan.directories.iter().any(|entry| entry.path == root) {
                plan.directories
                    .push(Entry::new(root.into()).map_err(|error| error.to_string())?);
            }
        }
        plan.check().map_err(|error| error.to_string())?;
        Ok(plan)
    }

    fn check(&self) -> io::Result<()> {
        for entry in self.directories.iter().chain(&self.sources) {
            entry
                .check()
                .map_err(|error| io::Error::other(format!("{}: {error}", name(&entry.path))))?;
        }
        let mut destinations = HashSet::new();
        for change in &self.changes {
            let (destination, case_change) = match change {
                Change::Rename { from, to } => (to, path_key(from) == path_key(to)),
                Change::Create { path, .. } => (path, false),
                _ => continue,
            };
            if !destinations.insert(path_key(destination)) {
                return Err(io::Error::other("multiple files have the same destination"));
            }
            match fs::symlink_metadata(destination) {
                Ok(_)
                    if case_change
                        && fs::canonicalize(destination).ok().as_deref() == Some(change.path()) => {
                }
                Ok(_) => {
                    return Err(io::Error::other(format!(
                        "destination exists: {}",
                        name(destination)
                    )));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub fn apply(&self) -> Result<(), String> {
        let _directories = self
            .directories
            .iter()
            .map(|entry| pin_directory(&entry.path))
            .collect::<io::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        self.check().map_err(|error| error.to_string())?;
        for (i, change) in self.changes.iter().enumerate() {
            let result = (|| {
                let parent = change.path().parent().unwrap();
                self.directories
                    .iter()
                    .find(|entry| entry.path == parent)
                    .unwrap()
                    .check()?;
                if let Some(source) = self
                    .sources
                    .iter()
                    .find(|entry| entry.path == change.path())
                {
                    source.check()?;
                }
                change.apply()
            })();
            if let Err(error) = result {
                let stranded = self.changes[..i]
                    .iter()
                    .rev()
                    .filter_map(|change| {
                        let Change::Rename { from, to } = change else {
                            return None;
                        };
                        move_exclusive(to, from).err().map(|_| paths::display(to))
                    })
                    .collect::<Vec<_>>();
                return Err(format!(
                    "{}: {error}{}",
                    paths::display(change.path()),
                    if stranded.is_empty() {
                        String::new()
                    } else {
                        format!("\n{}", stranded.join("\n"))
                    }
                ));
            }
        }
        Ok(())
    }

    pub fn preview(&self, catalog: &Catalog) -> String {
        let mut result = format!(
            "{}\n\n| {} | {} |\n| --- | --- |\n",
            catalog.get("files.preview"),
            catalog.get("files.before"),
            catalog.get("files.after"),
        );
        for change in &self.changes {
            let label = |path: &Path| {
                if self.directories.len() > 1 {
                    paths::display(path)
                } else {
                    name(path)
                }
            };
            let (before, after) = match change {
                Change::Rename { from, to } => (label(from), name(to)),
                Change::Create { path, .. } => (String::new(), label(path)),
                Change::Modify { path, .. } => (label(path), catalog.get("files.modified").into()),
                Change::Delete { path } => (label(path), catalog.get("files.deleted").into()),
            };
            result.push_str(&format!("| {} | {} |\n", cell(&before), cell(&after)));
        }
        for change in &self.changes {
            if let Change::Create { text, .. } | Change::Modify { text, .. } = change {
                let delimiter = if text.contains('`') {
                    "`".repeat(
                        text.split(|ch| ch != '`')
                            .map(str::len)
                            .max()
                            .unwrap_or(0)
                            .max(2)
                            + 1,
                    )
                } else {
                    "```".into()
                };
                result.push_str(&format!("\n{delimiter}\n{text}\n{delimiter}\n"));
            }
        }
        result.push_str(&format!("\n{}\n\n", catalog.get("files.directories")));
        for directory in &self.directories {
            result.push_str(&format!("- {}\n", cell(&paths::display(&directory.path))));
        }
        result.push_str(&format!("\n{}", catalog.get("files.confirm")));
        result
    }
}

impl Change {
    fn path(&self) -> &Path {
        match self {
            Self::Rename { from, .. } => from,
            Self::Create { path, .. } | Self::Modify { path, .. } | Self::Delete { path } => path,
        }
    }

    fn apply(&self) -> io::Result<()> {
        match self {
            Self::Rename { from, to } => move_exclusive(from, to),
            Self::Create { path, text } => {
                let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
                temporary.write_all(text.as_bytes())?;
                temporary
                    .persist_noclobber(path)
                    .map_err(|error| error.error)?;
                Ok(())
            }
            Self::Modify { path, text } => {
                let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
                let permissions = fs::metadata(path)?.permissions();
                temporary.write_all(text.as_bytes())?;
                temporary.as_file().set_permissions(permissions)?;
                temporary.persist(path).map_err(|error| error.error)?;
                Ok(())
            }
            Self::Delete { path } => fs::remove_file(path),
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn path_key(path: &Path) -> String {
    let path = paths::display(path);
    if cfg!(any(windows, target_os = "macos")) {
        path.to_lowercase()
    } else {
        path
    }
}

fn valid_name(name: &str) -> Result<&str, String> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| {
                matches!(
                    n,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
    {
        return Err("invalid file name".into());
    }
    Ok(name)
}

fn limited(text: String) -> Result<String, String> {
    if text.len() > TEXT_LIMIT {
        Err("text exceeds 64 KiB".into())
    } else {
        Ok(text)
    }
}

fn cell(text: &str) -> String {
    text.chars()
        .flat_map(|ch| match ch {
            '\n' => "␊".chars().collect(),
            '\r' => "␍".chars().collect(),
            ch if ch.is_ascii_punctuation() => vec!['\\', ch],
            ch => vec![ch],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn directory_scope_prevents_escaping_and_writes_only_on_apply() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new(vec![dir.path().to_owned()]).unwrap();
        let create = |name| json!({"operation":"create","directory":0,"name":name,"text":"hello"});
        assert!(Plan::propose(&scope, &create("../escape.txt")).is_err());
        assert!(Plan::propose(&scope, &create("stream:bad")).is_err());
        let plan = Plan::propose(&scope, &create("new.txt")).unwrap();
        let path = dir.path().join("new.txt");
        assert!(!path.exists());
        plan.apply().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        assert!(plan.apply().is_err());

        let scope = Scope::new(vec![path.clone()]).unwrap();
        assert!(Plan::propose(&scope, &create("another.txt")).is_err());
        let modify =
            Plan::propose(&scope, &json!({"operation":"modify","file":0,"text":"new"})).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
        modify.apply().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let scope = Scope::new(vec![path.clone()]).unwrap();
        let delete = Plan::propose(&scope, &json!({"operation":"delete","file":0})).unwrap();
        fs::write(&path, "changed after review").unwrap();
        assert!(delete.apply().is_err());
        let scope = Scope::new(vec![path.clone()]).unwrap();
        let delete = Plan::propose(&scope, &json!({"operation":"delete","file":0})).unwrap();
        delete.apply().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn rename_rejects_collisions_without_changing_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let one = dir.path().join("one.txt");
        let two = dir.path().join("two.txt");
        fs::write(&one, "one").unwrap();
        fs::write(&two, "two").unwrap();
        let scope = Scope::new(vec![one.clone(), two.clone()]).unwrap();
        let rename = |to| {
            json!({"operation":"rename","changes":[
                {"file":0,"name":"first.txt"},{"file":1,"name":to}
            ]})
        };
        assert!(Plan::propose(&scope, &rename("first.txt")).is_err());
        let plan = Plan::propose(&scope, &rename("second.txt")).unwrap();
        fs::write(dir.path().join("second.txt"), "occupied").unwrap();
        assert!(plan.apply().is_err());
        assert!(one.exists() && two.exists());
        assert!(!dir.path().join("first.txt").exists());
    }
}
