//! The global shortcuts the settings window edits: each search plugin's shortcut that opens
//! its scope, and the shortcuts of the commands it declares.
use crate::i18n;
use anyhow::{Result, ensure};
use mytools_plugin_api::{
    commands::CommandManifest,
    hotkeys::{Hotkey as Chord, LauncherHotkeys, launcher_config_path, normalize},
    plugin_settings::{CommandOverride, Override, Settings},
};
use std::collections::BTreeMap;

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

/// Validates only the edited key, so it is always possible to clear an existing binding.
pub fn validate_capture(
    entries: &[crate::plugins::Entry],
    index: usize,
    row: usize,
    text: &str,
) -> Result<String, String> {
    let current = entries
        .get(index)
        .and_then(|entry| entry.hotkeys.get(row))
        .ok_or_else(|| i18n::t("shortcut.invalid"))?;
    if text.is_empty() || text == current.hotkey {
        return Ok(text.into());
    }
    let chord = Chord::parse(text).ok_or_else(|| i18n::t("shortcut.invalid"))?;
    if !(chord.ctrl
        || chord.alt
        || chord.meta
        || (chord.key.starts_with('F') && chord.key.len() > 1))
    {
        return Err(i18n::t("shortcut.invalid"));
    }
    let normalized = chord.to_string();
    let config = launcher_hotkeys();
    let conflict = |owner: &str| {
        i18n::format(
            "shortcut.occupied",
            &[("shortcut", &normalized), ("name", owner)],
        )
    };
    for (global, label) in [
        (Some(config.launcher_hotkey.as_str()), "shortcut.launcher"),
        (config.selection(), "shortcut.selection"),
    ] {
        if global.and_then(Chord::parse).as_ref() == Some(&chord) {
            return Err(conflict(&i18n::t(label)));
        }
    }
    if chord.is_system_reserved() {
        return Err(i18n::format(
            "shortcut.reserved",
            &[("shortcut", &normalized)],
        ));
    }
    if normalized == "Ctrl+K" || normalized == "F1" {
        return Err(conflict(&i18n::t(if normalized == "Ctrl+K" {
            "shortcut.actions"
        } else {
            "shortcut.license"
        })));
    }
    for (other_index, entry) in entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.enabled || entry.locked())
    {
        for (other_row, hotkey) in entry.hotkeys.iter().enumerate() {
            if (other_index, other_row) != (index, row)
                && Chord::parse(&hotkey.hotkey).as_ref() == Some(&chord)
            {
                return Err(conflict(&format!("{} · {}", entry.name, hotkey.label)));
            }
        }
    }
    if let Some(owner) = action_owner(&chord) {
        return Err(conflict(&owner));
    }
    Ok(normalized)
}

fn action_owner(chord: &Chord) -> Option<String> {
    let data =
        std::fs::read(mytools_plugin_api::dirs::root().join("action-shortcuts.json")).ok()?;
    let bindings: Vec<serde_json::Value> = serde_json::from_slice(&data).ok()?;
    let binding = bindings
        .iter()
        .find(|value| value["shortcut"].as_str().and_then(Chord::parse).as_ref() == Some(chord))?;
    let target = &binding["target"];
    let owner = target["plugin"].as_str().unwrap_or("MyTools");
    let action = binding["title"]
        .as_str()
        .filter(|title| !title.is_empty())
        .or_else(|| target["command"].as_str())
        .or_else(|| target["action"].as_str());
    Some(action.map_or_else(|| owner.to_owned(), |action| format!("{owner} · {action}")))
}

/// Every shortcut must be valid, and those of enabled plugins must differ from each other
/// and from the launcher's.
pub fn validate<'a>(plugins: impl Iterator<Item = (&'a str, bool, &'a [Hotkey])>) -> Result<()> {
    validate_with(plugins, &launcher_hotkeys())
}

fn launcher_hotkeys() -> LauncherHotkeys {
    std::fs::read(launcher_config_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn validate_with<'a>(
    plugins: impl Iterator<Item = (&'a str, bool, &'a [Hotkey])>,
    hotkeys: &LauncherHotkeys,
) -> Result<()> {
    let reserved: Vec<_> = [Some(hotkeys.launcher_hotkey.as_str()), hotkeys.selection()]
        .into_iter()
        .flatten()
        .filter_map(normalize)
        .collect();
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
                !reserved.contains(&normalized),
                i18n::format("error.hotkey_launcher", &[("hotkey", &normalized)])
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
        assert!(
            validate_with(
                [("A", true, &one[..]), ("B", true, &same[..])].into_iter(),
                &LauncherHotkeys::default()
            )
            .is_err()
        );
        // A disabled plugin's shortcut is not registered, so it may repeat.
        assert!(
            validate_with(
                [("A", true, &one[..]), ("B", false, &same[..])].into_iter(),
                &LauncherHotkeys::default()
            )
            .is_ok()
        );
        let invalid = [hotkey(None, "", "Ctrl+Nope")];
        assert!(
            validate_with(
                [("A", false, &invalid[..])].into_iter(),
                &LauncherHotkeys::default()
            )
            .is_err()
        );
        let launcher = [hotkey(None, "", "alt+space")];
        assert!(
            validate_with(
                [("A", true, &launcher[..])].into_iter(),
                &LauncherHotkeys::default()
            )
            .is_err()
        );
        let custom = LauncherHotkeys {
            launcher_hotkey: "Ctrl+Space".into(),
            selection_hotkey: Some("Alt+X".into()),
        };
        assert!(validate_with([("A", true, &launcher[..])].into_iter(), &custom).is_ok());
        let claimed = [hotkey(None, "", "ctrl+space")];
        assert!(validate_with([("A", true, &claimed[..])].into_iter(), &custom).is_err());
        #[cfg(windows)]
        {
            let selection = [hotkey(None, "", "alt+x")];
            assert!(validate_with([("A", true, &selection[..])].into_iter(), &custom).is_err());
        }
        let empty = [hotkey(None, "Alt+V", " ")];
        assert!(
            validate_with(
                [("A", true, &empty[..])].into_iter(),
                &LauncherHotkeys::default()
            )
            .is_ok()
        );
    }
}
