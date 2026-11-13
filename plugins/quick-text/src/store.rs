use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Phrase {
    pub trigger: String,
    pub content: String,
}

pub struct Store {
    pub phrases: Vec<Phrase>,
    pub path: PathBuf,
    _lock: fs::File,
}

pub fn directory() -> Option<PathBuf> {
    Some(std::env::var_os("MYTOOLS_QUICK_TEXT_DIR").map_or_else(
        || mytools_plugin_api::dirs::plugin("quick-text"),
        PathBuf::from,
    ))
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("store.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|e| e.to_string())?;
        let path = root.join("phrases.json");
        let phrases = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice::<Vec<Phrase>>(&bytes).map_err(|e| e.to_string())?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(error) => return Err(error.to_string()),
        };
        Ok(Self {
            phrases,
            path,
            _lock: lock,
        })
    }

    pub fn commit(&mut self, phrases: Vec<Phrase>) -> Result<(), String> {
        use std::io::Write;
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&phrases).map_err(|e| e.to_string())?;
        let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(|e| e.to_string())?;
        self.phrases = phrases;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_multiline_content_and_refuses_concurrent_writers() {
        let root = std::env::temp_dir().join(format!("mytools-quick-test-{}", std::process::id()));
        let mut store = Store::open(root.clone()).unwrap();
        assert!(Store::open(root.clone()).is_err());
        let phrase = Phrase {
            trigger: "hello".into(),
            content: "中文\nworld".into(),
        };
        store.commit(vec![phrase.clone()]).unwrap();
        drop(store);
        let store = Store::open(root.clone()).unwrap();
        assert_eq!(store.phrases, vec![phrase]);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
