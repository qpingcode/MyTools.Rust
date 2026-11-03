use crate::i18n;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    Text { text: String, html: Option<String> },
    Image { width: u32, height: u32 },
    Files { paths: Vec<PathBuf> },
}

pub enum Captured {
    Text {
        text: String,
        html: Option<String>,
    },
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Files(Vec<PathBuf>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub timestamp: u64,
    pub bytes: u64,
    pub content: Content,
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub max_history_days: u64,
    pub max_history_count: usize,
    pub max_content_bytes: usize,
    pub max_total_bytes: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            max_history_days: 10,
            max_history_count: 500,
            max_content_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}

pub struct Store {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    settings: Settings,
    pub last_error: Option<String>,
    pub revision: u64,
    _lock: fs::File,
}

pub fn directory() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("MYTOOLS_CLIPBOARD_DIR") {
        return Ok(PathBuf::from(root));
    }
    #[cfg(windows)]
    return Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA missing"))?,
    )
    .join("MyTools/clipboard"));
    #[cfg(not(windows))]
    Ok(
        PathBuf::from(std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME missing"))?)
            .join("Library/Application Support/MyTools/clipboard"),
    )
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("instance.lock"))?;
        lock.try_lock()?;
        let settings = match fs::read(root.join("settings.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let settings = Settings::default();
                fs::write(
                    root.join("settings.json"),
                    serde_json::to_vec_pretty(&settings)?,
                )?;
                settings
            }
            Err(error) => return Err(error.into()),
        };
        let entries: Vec<Entry> = match fs::read(root.join("history.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let root = root.canonicalize()?;
        let mut store = Self {
            root,
            entries,
            settings,
            last_error: None,
            revision: now(),
            _lock: lock,
        };
        store.entries.retain(|entry| {
            entry.id.len() == 64 && entry.id.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let previous = store.entries.clone();
        store.prune();
        store.commit(previous)?;
        Ok(store)
    }

    pub fn image_path(&self, id: &str, preview: bool) -> PathBuf {
        self.root.join(if preview {
            format!("{id}.preview.png")
        } else {
            format!("{id}.png")
        })
    }

    pub fn add(&mut self, captured: Captured) -> Result<()> {
        let (content, payload) = match captured {
            Captured::Text { text, html } => {
                if text.is_empty() {
                    return Ok(());
                }
                let size = text.len() + html.as_ref().map_or(0, String::len);
                ensure!(
                    size <= self.settings.max_content_bytes.min(2 * 1024 * 1024),
                    i18n::text("too_large")
                );
                (Content::Text { text, html }, Vec::new())
            }
            Captured::Files(paths) => {
                if paths.is_empty() {
                    return Ok(());
                }
                ensure!(
                    paths.len() <= 1024 && paths.iter().all(|path| path.is_absolute()),
                    i18n::text("too_large")
                );
                (Content::Files { paths }, Vec::new())
            }
            Captured::Image {
                width,
                height,
                rgba,
            } => {
                ensure!(
                    rgba.len() <= self.settings.max_content_bytes
                        && rgba.len() as u64 == width as u64 * height as u64 * 4,
                    i18n::text("too_large")
                );
                (Content::Image { width, height }, rgba)
            }
        };
        let serialized = serde_json::to_vec(&content)?;
        ensure!(
            serialized.len() <= self.settings.max_content_bytes.min(2 * 1024 * 1024),
            i18n::text("too_large")
        );
        let mut hash = Sha256::new();
        hash.update(&serialized);
        hash.update(&payload);
        let id = format!("{:x}", hash.finalize());
        let mut bytes = serialized.len() as u64;
        let preview_text = match &content {
            Content::Text { text, .. } => Some(text.clone()),
            Content::Files { paths } => Some(
                paths
                    .iter()
                    .map(|path| mytools_plugin_api::paths::display(path))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        };
        if let Some(text) = preview_text {
            bytes += text.len() as u64;
            fs::write(self.root.join(format!("{id}.txt")), text)?;
        }
        if let Content::Image { width, height } = content {
            if !self.image_path(&id, false).exists() {
                let image = image::RgbaImage::from_raw(width, height, payload)
                    .ok_or_else(|| anyhow::anyhow!(i18n::text("too_large")))?;
                image.save_with_format(self.image_path(&id, false), image::ImageFormat::Png)?;
                image::DynamicImage::ImageRgba8(image)
                    .thumbnail(512, 512)
                    .save_with_format(self.image_path(&id, true), image::ImageFormat::Png)?;
            }
            bytes += fs::metadata(self.image_path(&id, false))?.len();
            bytes += fs::metadata(self.image_path(&id, true))?.len();
        }
        let previous = self.entries.clone();
        self.entries.retain(|entry| entry.id != id);
        self.entries.insert(
            0,
            Entry {
                id,
                timestamp: now(),
                bytes,
                content,
            },
        );
        self.prune();
        self.commit(previous)
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| anyhow::anyhow!(i18n::text("missing")))?;
        let previous = self.entries.clone();
        self.entries.remove(index);
        self.commit(previous)
    }

    pub fn clear(&mut self) -> Result<()> {
        let entries = std::mem::take(&mut self.entries);
        self.commit(entries)
    }

    pub fn cleanup(&mut self) -> Result<()> {
        let previous = self.entries.clone();
        self.prune();
        if previous.len() != self.entries.len() {
            self.commit(previous)?;
        }
        Ok(())
    }

    fn remove_images(&self, entry: &Entry) {
        let _ = fs::remove_file(self.root.join(format!("{}.txt", entry.id)));
        if matches!(entry.content, Content::Image { .. }) {
            let _ = fs::remove_file(self.image_path(&entry.id, false));
            let _ = fs::remove_file(self.image_path(&entry.id, true));
        }
    }

    fn prune(&mut self) {
        let cutoff =
            now().saturating_sub(self.settings.max_history_days.clamp(1, 3650) * 86_400_000);
        let mut size = 0u64;
        let mut kept = Vec::new();
        for entry in std::mem::take(&mut self.entries) {
            let next_size = size.saturating_add(entry.bytes);
            if entry.timestamp >= cutoff
                && kept.len() < self.settings.max_history_count.clamp(1, 5000)
                && next_size <= self.settings.max_total_bytes.max(1024)
            {
                size = next_size;
                kept.push(entry);
            }
        }
        self.entries = kept;
    }

    fn save(&self) -> Result<()> {
        let temporary = self.root.join("history.tmp");
        fs::write(&temporary, serde_json::to_vec(&self.entries)?)?;
        fs::rename(temporary, self.root.join("history.json"))?;
        Ok(())
    }

    fn commit(&mut self, previous: Vec<Entry>) -> Result<()> {
        if let Err(error) = self.save() {
            self.entries = previous;
            return Err(error);
        }
        for entry in previous {
            if !self.entries.iter().any(|current| current.id == entry.id) {
                self.remove_images(&entry);
            }
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "mytools-clipboard-{label}-{}-{}",
                std::process::id(),
                now()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn text(value: &str) -> Captured {
        Captured::Text {
            text: value.into(),
            html: None,
        }
    }

    #[test]
    fn persists_and_moves_duplicates_to_front_without_losing_content() {
        let root = Fixture::new("duplicates");
        let mut store = Store::open(root.0.clone()).unwrap();
        assert!(
            Store::open(root.0.clone()).is_err(),
            "a second writer must not open the same history"
        );
        store.add(text("first")).unwrap();
        let id = store.entries[0].id.clone();
        store.add(text("second")).unwrap();
        store.add(text("first")).unwrap();
        assert_eq!(store.entries.len(), 2);
        assert_eq!(store.entries[0].id, id);
        assert_eq!(
            fs::read_to_string(store.root.join(format!("{id}.txt"))).unwrap(),
            "first"
        );
        drop(store);
        let store = Store::open(root.0.clone()).unwrap();
        assert_eq!(store.entries[0].id, id);
    }

    #[test]
    fn stores_original_image_and_preview_then_deletes_assets() {
        let root = Fixture::new("image");
        let mut store = Store::open(root.0.clone()).unwrap();
        store
            .add(Captured::Image {
                width: 800,
                height: 600,
                rgba: vec![200; 800 * 600 * 4],
            })
            .unwrap();
        let id = store.entries[0].id.clone();
        assert_eq!(
            image::image_dimensions(store.image_path(&id, false)).unwrap(),
            (800, 600)
        );
        let (width, height) = image::image_dimensions(store.image_path(&id, true)).unwrap();
        assert!(width <= 512 && height <= 512);
        store.delete(&id).unwrap();
        assert!(!store.image_path(&id, false).exists());
        assert!(!store.image_path(&id, true).exists());
        drop(store);
        assert!(Store::open(root.0.clone()).unwrap().entries.is_empty());
    }

    #[test]
    fn enforces_retention_and_preserves_history_when_commit_fails() {
        let root = Fixture::new("retention");
        fs::write(root.0.join("settings.json"), r#"{"max_history_count":2}"#).unwrap();
        let mut store = Store::open(root.0.clone()).unwrap();
        for value in ["a", "b", "c"] {
            store.add(text(value)).unwrap();
        }
        assert_eq!(store.entries.len(), 2);
        assert!(matches!(&store.entries[0].content, Content::Text { text, .. } if text == "c"));
        let id = store.entries[0].id.clone();
        fs::create_dir(store.root.join("history.tmp")).unwrap();
        assert!(store.delete(&id).is_err());
        assert_eq!(store.entries.len(), 2);
        assert!(store.root.join(format!("{id}.txt")).exists());
        fs::remove_dir(store.root.join("history.tmp")).unwrap();
        store.entries[1].timestamp = 0;
        store.save().unwrap();
        drop(store);
        let mut store = Store::open(root.0.clone()).unwrap();
        assert_eq!(store.entries.len(), 1);
        store.clear().unwrap();
        assert!(store.entries.is_empty());
    }
}
