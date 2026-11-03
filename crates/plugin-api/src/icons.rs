//! UI-independent icon references. Existing string icons stay wire-compatible.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Icon {
    Name(String),
    Native { native: PathBuf },
}

impl From<String> for Icon {
    fn from(value: String) -> Self {
        Self::Name(value)
    }
}

impl From<&str> for Icon {
    fn from(value: &str) -> Self {
        Self::Name(value.into())
    }
}

pub fn resolve_reference(icon: Option<&Icon>, fallback: &Icon) -> Icon {
    fn valid_reference(icon: &Icon) -> bool {
        match icon {
            Icon::Name(name) => valid(name),
            Icon::Native { native } => {
                native.is_absolute() && !native.as_os_str().as_encoded_bytes().contains(&0)
            }
        }
    }
    icon.filter(|icon| valid_reference(icon))
        .or_else(|| valid_reference(fallback).then_some(fallback))
        .cloned()
        .unwrap_or_else(|| DEFAULT_ICON.into())
}

pub const BUILTIN_ICON_NAMES: &str = include_str!("../../../ui/assets/icons/names.txt");
pub const DEFAULT_ICON: &str = "builtin:puzzle";

fn valid(icon: &str) -> bool {
    if let Some(name) = icon.strip_prefix("builtin:") {
        BUILTIN_ICON_NAMES
            .lines()
            .any(|candidate| candidate == name)
    } else {
        !icon.trim().is_empty()
    }
}

/// Prefer an explicit icon, then its fallback, then the generic plugin icon.
pub fn resolve(icon: Option<&str>, fallback: &str) -> String {
    icon.filter(|icon| valid(icon))
        .or_else(|| valid(fallback).then_some(fallback))
        .unwrap_or(DEFAULT_ICON)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_icon_references_round_trip_and_reject_invalid_paths() {
        let icon = Icon::Native {
            native: std::env::temp_dir().join("图标 app.lnk"),
        };
        let json = serde_json::to_string(&icon).unwrap();
        assert_eq!(serde_json::from_str::<Icon>(&json).unwrap(), icon);
        assert_eq!(resolve_reference(Some(&icon), &DEFAULT_ICON.into()), icon);
        for native in [
            PathBuf::from("relative.lnk"),
            std::env::temp_dir().join("bad\0path"),
        ] {
            assert_eq!(
                resolve_reference(Some(&Icon::Native { native }), &"builtin:calculator".into()),
                "builtin:calculator".into()
            );
        }
        assert_eq!(
            serde_json::from_str::<Icon>(r#""builtin:calculator""#).unwrap(),
            "builtin:calculator".into()
        );
    }

    #[test]
    fn resolves_builtin_text_and_invalid_icons() {
        assert_eq!(
            resolve(Some("builtin:folder"), DEFAULT_ICON),
            "builtin:folder"
        );
        assert_eq!(resolve(Some("＝"), DEFAULT_ICON), "＝");
        for icon in [None, Some(""), Some("  "), Some("builtin:missing")] {
            assert_eq!(resolve(icon, "builtin:calculator"), "builtin:calculator");
        }
        assert_eq!(
            resolve(Some("builtin:missing"), "builtin:bad"),
            DEFAULT_ICON
        );
    }
}
