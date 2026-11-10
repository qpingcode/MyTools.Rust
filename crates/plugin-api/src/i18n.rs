//! Small JSON catalogs shared by hosts and plugins; no UI dependencies.
use std::{collections::HashMap, sync::OnceLock};

static LANGUAGE: OnceLock<String> = OnceLock::new();

/// Resolve supported languages. Traditional Chinese falls back to English for now.
pub fn resolve(requested: &str, system: Option<&str>) -> &'static str {
    let source = if requested == "auto" {
        system.unwrap_or("en")
    } else {
        requested
    };
    let source = source.replace('_', "-").to_ascii_lowercase();
    if source == "zh"
        || source.starts_with("zh-cn")
        || source.starts_with("zh-sg")
        || source.starts_with("zh-hans")
    {
        "zh-CN"
    } else {
        "en"
    }
}

/// Call once at startup, before starting workers or creating UI components.
pub fn initialize(requested: &str) -> &'static str {
    LANGUAGE.get_or_init(|| resolve(requested, sys_locale::get_locale().as_deref()).into())
}

pub fn language() -> &'static str {
    LANGUAGE.get_or_init(|| {
        let inherited = std::env::var("MYTOOLS_LOCALE").unwrap_or_else(|_| "auto".into());
        resolve(&inherited, sys_locale::get_locale().as_deref()).into()
    })
}

pub struct Catalog {
    selected: HashMap<String, String>,
    english: HashMap<String, String>,
}

/// A plugin's text from its `locales/<language>.json`, falling back to English and then
/// to `default`. Missing locale files are not an error.
pub fn plugin_text(
    plugin_directory: &std::path::Path,
    key: &str,
    default: &str,
    language: &str,
) -> std::io::Result<String> {
    let read = |language: &str| -> std::io::Result<String> {
        match std::fs::read_to_string(plugin_directory.join(format!("locales/{language}.json"))) {
            Ok(json) => Ok(json),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("{}".into()),
            Err(error) => Err(error),
        }
    };
    let catalog =
        Catalog::from_json(&read(language)?, &read("en")?).map_err(std::io::Error::other)?;
    Ok(catalog.lookup(key).unwrap_or(default).into())
}

impl Catalog {
    pub fn for_language(english: &str, chinese: &str) -> serde_json::Result<Self> {
        Self::from_json(
            if language() == "zh-CN" {
                chinese
            } else {
                english
            },
            english,
        )
    }

    pub fn from_json(selected: &str, english: &str) -> serde_json::Result<Self> {
        Ok(Self {
            selected: serde_json::from_str(selected)?,
            english: serde_json::from_str(english)?,
        })
    }

    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.selected
            .get(key)
            .or_else(|| self.english.get(key))
            .map(String::as_str)
            .unwrap_or(key)
    }

    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.selected
            .get(key)
            .or_else(|| self.english.get(key))
            .map(String::as_str)
    }

    /// Substitute named values in one pass so values containing braces stay literal.
    pub fn format(&self, key: &str, values: &[(&str, &str)]) -> String {
        let template = self.get(key);
        let mut result = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            result.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}').map(|end| start + end) else {
                result.push_str(&rest[start..]);
                return result;
            };
            let name = &rest[start + 1..end];
            result.push_str(
                values
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| *value)
                    .unwrap_or(&rest[start..=end]),
            );
            rest = &rest[end + 1..];
        }
        result.push_str(rest);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_system_language_and_unsupported_locales() {
        assert_eq!(resolve("auto", Some("zh_CN")), "zh-CN");
        assert_eq!(resolve("auto", Some("zh-Hans-CN")), "zh-CN");
        assert_eq!(resolve("en", Some("zh-CN")), "en");
        assert_eq!(resolve("auto", None), "en");
        assert_eq!(resolve("auto", Some("zh-TW")), "en");
        assert_eq!(resolve("fr", None), "en");
    }

    #[test]
    fn falls_back_and_preserves_interpolated_user_content() {
        let catalog = Catalog::from_json(
            r#"{"copied":"已复制 {text}","text":"不应替换"}"#,
            r#"{"missing":"Fallback"}"#,
        )
        .unwrap();
        assert_eq!(catalog.get("missing"), "Fallback");
        assert_eq!(catalog.get("unknown"), "unknown");
        assert_eq!(
            catalog.format("copied", &[("text", "file {text}.txt")]),
            "已复制 file {text}.txt"
        );
        assert_eq!(catalog.format("copied", &[]), "已复制 {text}");
        assert!(Catalog::from_json("invalid", "{}").is_err());
    }
}
