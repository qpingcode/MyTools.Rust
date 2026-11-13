//! Finished conversations, one JSON file each, and the model picked with Ctrl+K. They live
//! in the plugin's folder, `%LOCALAPPDATA%\MyTools\ask-ai` (`MYTOOLS_ASK_AI_DATA` elsewhere);
//! the newest `limit` conversations are kept.
use mytools_plugin_api::chat::{Chat, Role};
use std::path::PathBuf;

#[derive(Clone)]
pub struct History {
    /// `None` keeps nothing, e.g. without a data folder or with a limit of 0.
    dir: Option<PathBuf>,
    limit: usize,
}

fn default_dir() -> Option<PathBuf> {
    Some(
        std::env::var_os("MYTOOLS_ASK_AI_DATA")
            .map_or_else(crate::config::directory, PathBuf::from),
    )
}

/// Ids are made by the plugin from digits and `-`; anything else is not a file of ours.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit() || ch == '-')
}

impl History {
    pub fn open(limit: usize) -> Self {
        Self {
            dir: default_dir(),
            limit,
        }
    }

    #[cfg(test)]
    pub fn temporary() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ask-ai-history-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Self {
            dir: Some(dir),
            limit: 3,
        }
    }

    fn chats(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("chats"))
    }

    /// Keeps `chat` unless it is empty, then drops the oldest beyond the limit.
    pub fn save(&self, chat: &Chat) {
        let Some(chats) = self.chats() else { return };
        if self.limit == 0 || chat.messages.is_empty() || !valid_id(&chat.id) {
            return;
        }
        if std::fs::create_dir_all(&chats).is_err() {
            return;
        }
        let saved = Chat {
            attachment: None,
            ..chat.clone()
        };
        let Ok(bytes) = serde_json::to_vec(&saved) else {
            return;
        };
        let path = chats.join(format!("{}.json", chat.id));
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(&temporary, &path);
        }
        for (id, _) in self.files().into_iter().skip(self.limit) {
            let _ = std::fs::remove_file(chats.join(format!("{id}.json")));
        }
    }

    pub fn load(&self, id: &str) -> Option<Chat> {
        if !valid_id(id) {
            return None;
        }
        let bytes = std::fs::read(self.chats()?.join(format!("{id}.json"))).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Saved conversations, newest first, by id.
    fn files(&self) -> Vec<(String, std::time::SystemTime)> {
        let Some(chats) = self.chats() else {
            return Vec::new();
        };
        let mut files: Vec<_> = std::fs::read_dir(chats)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let id = path
                    .file_name()?
                    .to_str()?
                    .strip_suffix(".json")?
                    .to_owned();
                let modified = entry.metadata().ok()?.modified().ok()?;
                valid_id(&id).then_some((id, modified))
            })
            .collect();
        files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        files
    }

    /// The newest `count` conversations with their titles (the first question).
    pub fn recent(&self, count: usize) -> Vec<(String, String)> {
        self.files()
            .into_iter()
            .take(count)
            .filter_map(|(id, _)| {
                let chat = self.load(&id)?;
                Some((id, title(&chat)))
            })
            .collect()
    }

    fn model_file(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("model.txt"))
    }

    pub fn model(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.model_file()?).ok()?;
        Some(text.trim().to_owned()).filter(|model| !model.is_empty())
    }

    pub fn set_model(&self, model: Option<&str>) {
        let Some(file) = self.model_file() else {
            return;
        };
        match model {
            Some(model) => {
                let _ = file.parent().map(std::fs::create_dir_all);
                let _ = std::fs::write(file, model);
            }
            None => {
                let _ = std::fs::remove_file(file);
            }
        }
    }

    #[cfg(test)]
    pub fn dir(&self) -> &std::path::Path {
        self.dir.as_deref().unwrap()
    }
}

/// The first question, shortened to one line.
pub fn title(chat: &Chat) -> String {
    let question = chat
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .map(|message| {
            if message.text.trim().is_empty() {
                message.attachment.as_deref().unwrap_or_default()
            } else {
                message.text.as_str()
            }
        })
        .unwrap_or_default();
    let line = question.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(40) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mytools_plugin_api::chat::ChatMessage;

    fn chat(id: &str, question: &str) -> Chat {
        Chat {
            id: id.into(),
            messages: vec![ChatMessage {
                role: Role::User,
                text: question.into(),
                ..Default::default()
            }],
            attachment: Some("draft".into()),
        }
    }

    #[test]
    fn keeps_the_newest_conversations_and_the_model() {
        let history = History::temporary();
        history.save(&Chat {
            id: "1-1".into(),
            ..Default::default()
        });
        assert!(
            history.recent(5).is_empty(),
            "empty conversations are not kept"
        );
        for (index, id) in ["1-1", "1-2", "1-3", "1-4"].iter().enumerate() {
            history.save(&chat(id, &format!("question {index}")));
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let recent = history.recent(5);
        assert_eq!(recent.len(), 3, "the oldest beyond the limit is dropped");
        assert_eq!(recent[0], ("1-4".into(), "question 3".into()));
        assert_eq!(history.load("1-4").unwrap().attachment, None);
        assert!(history.load("../x").is_none());
        assert_eq!(history.model(), None);
        history.set_model(Some("m"));
        assert_eq!(history.model().as_deref(), Some("m"));
        history.set_model(None);
        assert_eq!(history.model(), None);
        let _ = std::fs::remove_dir_all(history.dir());
    }

    #[test]
    fn titles_are_one_short_line() {
        assert_eq!(title(&chat("1", "  a\n b ")), "a b");
        assert_eq!(title(&chat("1", &"x".repeat(50))).chars().count(), 41);
    }
}
