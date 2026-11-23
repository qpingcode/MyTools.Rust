//! What the manager window edits: the saved scripts plus unsaved drafts of any of them,
//! including new scripts, so several can be edited before saving them together.
use crate::store::{Mode, Script};

pub struct Editor {
    /// The scripts as they are in the store.
    pub saved: Vec<Script>,
    /// Edited copies of saved scripts and new scripts, at most one per id.
    drafts: Vec<Script>,
    /// The id of the script shown in the form.
    pub current: Option<String>,
    pub sort_by_name: bool,
}

impl Editor {
    pub fn new(saved: Vec<Script>, sort_by_name: bool) -> Self {
        Self {
            saved,
            drafts: Vec::new(),
            current: None,
            sort_by_name,
        }
    }

    /// The saved version of a script, `None` for a new one.
    pub fn original(&self, id: &str) -> Option<&Script> {
        self.saved.iter().find(|s| s.id == id)
    }

    /// The script as last edited: its draft, else its saved version.
    pub fn get(&self, id: &str) -> Option<&Script> {
        self.drafts
            .iter()
            .find(|s| s.id == id)
            .or_else(|| self.original(id))
    }

    /// The values a script is compared with: its saved version, or a blank new script.
    pub fn baseline(&self, id: &str) -> Script {
        self.original(id).cloned().unwrap_or_else(|| {
            let created = self.get(id).map_or(0, |s| s.created);
            blank(id.into(), created)
        })
    }

    /// Starts a new blank script and makes it current.
    pub fn create(&mut self, created: u64) -> String {
        let script = blank(uuid::Uuid::new_v4().to_string(), created);
        let id = script.id.clone();
        self.drafts.push(script);
        self.current = Some(id.clone());
        id
    }

    /// Records the form's contents for its script. A copy equal to the saved version is
    /// dropped, and so is a new script left blank unless it is still the one being edited.
    pub fn put(&mut self, script: Script) {
        let keep = self.differs(&script)
            || (self.original(&script.id).is_none()
                && self.current.as_deref() == Some(script.id.as_str()));
        self.drafts.retain(|s| s.id != script.id);
        if keep {
            self.drafts.push(script);
        }
    }

    /// Makes another script current, dropping the previous one if it was new and left blank.
    pub fn select(&mut self, id: Option<String>) {
        let previous = std::mem::replace(&mut self.current, id);
        if let Some(previous) = previous
            && let Some(draft) = self.drafts.iter().find(|s| s.id == previous).cloned()
        {
            self.put(draft);
        }
    }

    /// Forgets the unsaved changes of a script; a new one disappears.
    pub fn discard(&mut self, id: &str) {
        self.drafts.retain(|s| s.id != id);
    }

    pub fn is_dirty(&self, id: &str) -> bool {
        self.drafts.iter().any(|s| s.id == id && self.differs(s))
    }

    /// The changed scripts other than `except`.
    pub fn dirty_count(&self, except: Option<&str>) -> usize {
        self.drafts
            .iter()
            .filter(|s| Some(s.id.as_str()) != except && self.differs(s))
            .count()
    }

    /// The changed scripts, trimmed for saving, each with the version it was edited from.
    pub fn changes(&self) -> Vec<(Script, Option<Script>)> {
        self.drafts
            .iter()
            .filter(|s| self.differs(s))
            .map(|s| {
                let mut script = s.clone();
                script.name = script.name.trim().into();
                script.alias = script.alias.trim().into();
                script.directory = script.directory.trim().into();
                (script, self.original(&s.id).cloned())
            })
            .collect()
    }

    /// Takes the store's scripts after a save; drafts that were saved are dropped.
    pub fn saved_all(&mut self, saved: Vec<Script>) {
        self.saved = saved;
        let drafts = std::mem::take(&mut self.drafts);
        self.drafts = drafts
            .into_iter()
            .filter(|s| self.original(&s.id).is_none() && !self.differs(s))
            .collect();
    }

    /// The scripts in the order the sidebar lists them, drafts in place of saved versions.
    pub fn rows(&self) -> Vec<&Script> {
        let new = self
            .drafts
            .iter()
            .filter(|s| self.original(&s.id).is_none());
        let mut rows: Vec<(usize, &Script)> = self
            .saved
            .iter()
            .map(|s| self.get(&s.id).unwrap_or(s))
            .chain(new)
            .enumerate()
            .collect();
        if self.sort_by_name {
            rows.sort_by_cached_key(|(_, s)| s.name.trim().to_lowercase());
        } else {
            // Newest first; scripts without a creation time keep their order in the store,
            // where later ones were added later.
            rows.sort_by_key(|(index, s)| std::cmp::Reverse((s.created, *index)));
        }
        rows.into_iter().map(|(_, s)| s).collect()
    }

    fn differs(&self, script: &Script) -> bool {
        *script != self.baseline(&script.id)
    }
}

/// A new script with nothing filled in, in the preferred PowerShell.
fn blank(id: String, created: u64) -> Script {
    Script {
        id,
        name: String::new(),
        alias: String::new(),
        kind: crate::runner::preferred_kind(),
        content: String::new(),
        directory: String::new(),
        mode: Mode::default(),
        created,
    }
}

/// Milliseconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(name: &str, created: u64) -> Script {
        let mut script = blank(uuid::Uuid::new_v4().to_string(), created);
        script.name = name.into();
        script.content = "echo".into();
        script
    }
    fn names(editor: &Editor) -> Vec<&str> {
        editor.rows().iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn sorts_by_name_or_newest_first() {
        let mut editor = Editor::new(
            vec![script("beta", 0), script("Alpha", 0), script("gamma", 5)],
            true,
        );
        assert_eq!(names(&editor), ["Alpha", "beta", "gamma"]);
        editor.sort_by_name = false;
        assert_eq!(names(&editor), ["gamma", "Alpha", "beta"]);
        let id = editor.create(9);
        let mut new = editor.get(&id).unwrap().clone();
        new.name = "delta".into();
        editor.put(new);
        assert_eq!(names(&editor), ["delta", "gamma", "Alpha", "beta"]);
    }

    #[test]
    fn keeps_drafts_of_several_scripts_until_saved() {
        let (first, second) = (script("one", 1), script("two", 2));
        let mut editor = Editor::new(vec![first.clone(), second.clone()], true);
        editor.select(Some(first.id.clone()));
        let mut edited = first.clone();
        edited.name = " uno ".into();
        editor.put(edited);
        editor.select(Some(second.id.clone()));
        assert!(editor.is_dirty(&first.id) && !editor.is_dirty(&second.id));
        assert_eq!(editor.dirty_count(None), 1);
        let changes = editor.changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].0.name, "uno");
        assert_eq!(changes[0].1.as_ref(), Some(&first));
        // Editing back to the saved values is no change at all.
        editor.put(second.clone());
        assert!(!editor.is_dirty(&second.id));
        editor.discard(&first.id);
        assert_eq!(editor.dirty_count(None), 0);
    }

    #[test]
    fn a_blank_new_script_disappears_once_left() {
        let mut editor = Editor::new(vec![script("one", 1)], true);
        let id = editor.create(2);
        assert_eq!(editor.rows().len(), 2);
        assert!(!editor.is_dirty(&id));
        editor.select(None);
        assert_eq!(editor.rows().len(), 1);
        let id = editor.create(3);
        let mut filled = editor.get(&id).unwrap().clone();
        filled.content = "echo".into();
        editor.put(filled.clone());
        editor.select(None);
        assert!(editor.is_dirty(&id));
        editor.saved_all(vec![script("one", 1), filled]);
        assert_eq!(editor.dirty_count(None), 0);
    }
}
