use mytools_plugin_api::{Resource, SearchItem, i18n::Catalog};
use serde_json::Value;
use std::{
    cell::RefCell,
    fs,
    io::Read,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Bookmark {
    title: String,
    url: String,
    folder: String,
}

fn chrome_directory() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MYTOOLS_CHROME_DIR") {
        return Some(path.into());
    }
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("Google/Chrome/User Data"));
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME")
        .map(|root| PathBuf::from(root).join("Library/Application Support/Google/Chrome"));
    #[cfg(not(any(windows, target_os = "macos")))]
    std::env::var_os("HOME").map(|root| PathBuf::from(root).join(".config/google-chrome"))
}

fn collect(node: &Value, folder: &str, output: &mut Vec<Bookmark>) {
    let name = node["name"].as_str().unwrap_or_default();
    if let Some(url) = node["url"].as_str() {
        // Only web URLs are opened; bookmarks cannot invoke local scripts or executables.
        if (url.starts_with("https://") || url.starts_with("http://"))
            && url.len() <= 4096
            && name.len() <= 1024
            && folder.len() <= 2048
        {
            output.push(Bookmark {
                title: name.into(),
                url: url.into(),
                folder: folder.into(),
            });
        }
    }
    if let Some(children) = node["children"].as_array() {
        let folder = if folder.is_empty() {
            name.into()
        } else {
            format!("{folder} / {name}")
        };
        for child in children {
            collect(child, &folder, output);
        }
    }
}

fn load() -> Vec<Bookmark> {
    let mut output = vec![];
    let Some(root) = chrome_directory() else {
        return output;
    };
    let Ok(profiles) = fs::read_dir(root) else {
        return output;
    };
    for profile in profiles.flatten() {
        let name = profile.file_name().to_string_lossy().into_owned();
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }
        let Ok(file) = fs::File::open(profile.path().join("Bookmarks")) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > 8 * 1024 * 1024
        {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Some(roots) = value["roots"].as_object() {
            for node in roots.values() {
                collect(node, &name, &mut output);
            }
        }
    }
    output.sort_by_key(|entry| entry.title.to_lowercase());
    output
}

fn search(entries: &[Bookmark], query: &str, catalog: &Catalog) -> Vec<SearchItem> {
    let words: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    entries
        .iter()
        .filter(|entry| {
            let searchable =
                format!("{} {} {}", entry.title, entry.url, entry.folder).to_lowercase();
            mytools_plugin_api::matching::rank(&entry.title, query) < 4
                || words.iter().all(|word| searchable.contains(word))
        })
        .take(50)
        .map(|entry| SearchItem {
            intent_match: false,
            title: entry.title.clone(),
            description: catalog.format(
                "bookmark.description",
                &[("folder", &entry.folder), ("url", &entry.url)],
            ),
            icon: None,
            resource: Resource::Url {
                url: entry.url.clone(),
            },
            actions: None,
            preview: None,
        })
        .collect()
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    let cache = RefCell::new((Instant::now(), load()));
    mytools_plugin_api::serve(|query| {
        let mut cache = cache.borrow_mut();
        if cache.0.elapsed() > Duration::from_secs(5) {
            *cache = (Instant::now(), load());
        }
        search(&cache.1, query, &catalog)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_bookmarks_keep_folders_and_only_open_web_links() {
        let value = serde_json::json!({"name":"Work","children":[
            {"name":"Rust","url":"https://rust-lang.org"},
            {"name":"Unsafe","url":"javascript:alert(1)"}]});
        let mut entries = vec![];
        collect(&value, "Default", &mut entries);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].folder, "Default / Work");
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        assert_eq!(search(&entries, "work RUST", &catalog).len(), 1);
        assert!(search(&entries, "missing", &catalog).is_empty());
    }
}
