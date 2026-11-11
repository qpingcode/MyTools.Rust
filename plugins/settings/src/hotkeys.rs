//! The global shortcuts the settings window edits: each search plugin's shortcut that opens
//! its scope, and the shortcuts of the commands it declares.
use crate::i18n;
use anyhow::{Result, ensure};
use mytools_plugin_api::{
    commands::CommandManifest,
    hotkeys::normalize,
    plugin_settings::{CommandOverride, Override, Settings},
};
use std::collections::BTreeMap;

/// The launcher's own shortcut, which no plugin may take.
const LAUNCHER: &str = "Alt+Space";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    /// The command this shortcut runs; `None` for the one that opens the plugin's scope.
    pub command: Option<String>,
    pub label: String,
    /// The manifest's shortcut, "" for none.
    pub default: String,
    /// As edited, "" for none.
    pub hotkey: String,
}

/// Shows valid shortcuts in one spelling and keeps anything else as written.
fn display(text: Option<&str>) -> String {
    let text = text.unwrap_or_default();
    normalize(text).unwrap_or_else(|| text.to_owned())
}

/// The shortcuts of one search plugin with the user's overrides applied.
pub fn list(
    id: &str,
    manifest_hotkey: Option<&str>,
    commands: &[CommandManifest],
    settings: &Settings,
    title: impl Fn(&CommandManifest) -> String,
) -> Vec<Hotkey> {
    let mut hotkeys = vec![Hotkey {
        command: None,
        label: i18n::t("hotkeys.scope"),
        default: display(manifest_hotkey),
        hotkey: display(settings.hotkey(id, manifest_hotkey)),
    }];
    hotkeys.extend(commands.iter().map(|command| Hotkey {
        command: Some(command.id.clone()),
        label: title(command),
        default: display(command.hotkey.as_deref()),
        hotkey: display(settings.command_hotkey(id, &command.id, command.hotkey.as_deref())),
    }));
    hotkeys
}

/// Writes the shortcuts that differ from the manifest into a plugin's override.
pub fn apply(hotkeys: &[Hotkey], item: &mut Override) {
    let changed = |hotkey: &Hotkey| {
        let edited = normalize(&hotkey.hotkey).unwrap_or_default();
        (edited != normalize(&hotkey.default).unwrap_or_default()).then_some(edited)
    };
    item.hotkey = None;
    item.commands = BTreeMap::new();
    for hotkey in hotkeys {
        let Some(edited) = changed(hotkey) else {
            continue;
        };
        match &hotkey.command {
            None => item.hotkey = Some(edited),
            Some(command) => {
                item.commands.insert(
                    command.clone(),
                    CommandOverride {
                        hotkey: Some(edited),
                    },
                );
            }
        }
    }
}

/// Every shortcut must be valid, and those of enabled plugins must differ from each other
/// and from the launcher's.
pub fn validate<'a>(plugins: impl Iterator<Item = (&'a str, bool, &'a [Hotkey])>) -> Result<()> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for (name, enabled, hotkeys) in plugins {
        for hotkey in hotkeys {
            let owner = match &hotkey.command {
                None => name.to_owned(),
                Some(_) => format!("{name} · {}", hotkey.label),
            };
            let normalized = normalize(&hotkey.hotkey);
            let Some(normalized) = normalized.filter(|text| !text.is_empty()) else {
                ensure!(
                    hotkey.hotkey.trim().is_empty(),
                    i18n::format(
                        "error.hotkey",
                        &[("name", &owner), ("hotkey", hotkey.hotkey.trim())]
                    )
                );
                continue;
            };
            if !enabled {
                continue;
            }
            ensure!(
                normalized != LAUNCHER,
                i18n::format("error.hotkey_launcher", &[("hotkey", LAUNCHER)])
            );
            if let Some((_, first)) = seen.iter().find(|(known, _)| *known == normalized) {
                anyhow::bail!(i18n::format(
                    "error.duplicate_hotkey",
                    &[
                        ("hotkey", &normalized),
                        ("first", first),
                        ("second", &owner)
                    ]
                ));
            }
            seen.push((normalized, owner));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hotkey(command: Option<&str>, default: &str, edited: &str) -> Hotkey {
        Hotkey {
            command: command.map(str::to_owned),
            label: command.unwrap_or("scope").into(),
            default: default.into(),
            hotkey: edited.into(),
        }
    }

    #[test]
    fn saves_only_changed_shortcuts_in_canonical_form() {
        let mut item = Override::default();
        apply(
            &[
                hotkey(None, "Ctrl+Shift+V", "ctrl + shift+v"),
                hotkey(Some("paste-plain"), "Ctrl+Shift+Alt+V", ""),
                hotkey(Some("other"), "", "alt+f2"),
            ],
            &mut item,
        );
        assert_eq!(item.hotkey, None);
        assert_eq!(item.commands["paste-plain"].hotkey.as_deref(), Some(""));
        assert_eq!(item.commands["other"].hotkey.as_deref(), Some("Alt+F2"));
        apply(&[hotkey(None, "Alt+V", "Alt+T")], &mut item);
        assert_eq!(item.hotkey.as_deref(), Some("Alt+T"));
        assert!(item.commands.is_empty());
    }

    #[test]
    fn rejects_invalid_duplicate_and_reserved_shortcuts() {
        let one = [hotkey(None, "", "Ctrl+Shift+V")];
        let same = [hotkey(Some("x"), "", "shift+ctrl+v")];
        assert!(validate([("A", true, &one[..]), ("B", true, &same[..])].into_iter()).is_err());
        // A disabled plugin's shortcut is not registered, so it may repeat.
        assert!(validate([("A", true, &one[..]), ("B", false, &same[..])].into_iter()).is_ok());
        let invalid = [hotkey(None, "", "Ctrl+Nope")];
        assert!(validate([("A", false, &invalid[..])].into_iter()).is_err());
        let launcher = [hotkey(None, "", "alt+space")];
        assert!(validate([("A", true, &launcher[..])].into_iter()).is_err());
        let empty = [hotkey(None, "Alt+V", " ")];
        assert!(validate([("A", true, &empty[..])].into_iter()).is_ok());
    }
}
