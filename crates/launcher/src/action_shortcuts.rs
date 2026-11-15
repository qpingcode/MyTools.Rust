use crate::i18n;
use mytools_core::Action;
use mytools_plugin_api::{ActionId, dirs, hotkeys::Hotkey};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "owner", rename_all = "snake_case")]
pub enum Target {
    MyTools {
        action: ActionId,
    },
    Plugin {
        plugin: String,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<String>,
    },
}

impl Target {
    pub fn for_action(action: &Action, actions: &[Action]) -> Option<Self> {
        if action.id != ActionId::Plugin {
            return Some(Self::MyTools { action: action.id });
        }
        let (plugin, command, item) = action.plugin_command()?;
        let multiple = actions
            .iter()
            .filter_map(Action::plugin_command)
            .filter(|(owner, name, _)| *owner == plugin && *name == command)
            .take(2)
            .count()
            > 1;
        Some(Self::Plugin {
            plugin: plugin.into(),
            command: command.into(),
            item: if multiple {
                Some(item?.to_owned())
            } else {
                None
            },
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Binding {
    target: Target,
    shortcut: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
}

impl Binding {
    fn owner(&self) -> String {
        let (group, fallback) = match &self.target {
            Target::MyTools { action } => (
                "MyTools",
                i18n::t(match action {
                    ActionId::Open => "actions.open",
                    ActionId::OpenInBrowser => "actions.open_browser",
                    ActionId::CopyText => "actions.copy",
                    ActionId::CopyPath => "actions.copy_path",
                    ActionId::CopyName => "actions.copy_name",
                    ActionId::Reveal => "actions.reveal",
                    ActionId::Launch => "actions.launch",
                    ActionId::RunAsAdmin => "actions.run_as_admin",
                    ActionId::Plugin => "actions.execute",
                }),
            ),
            Target::Plugin {
                plugin, command, ..
            } => (plugin.as_str(), command.clone()),
        };
        format!(
            "{group} · {}",
            if self.title.is_empty() {
                &fallback
            } else {
                &self.title
            }
        )
    }
}

fn occupied(hotkey: &Hotkey, owner: &str) -> String {
    i18n::format(
        "actions.shortcut_occupied",
        &[("shortcut", &hotkey.to_string()), ("name", owner)],
    )
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Shortcuts(Vec<Binding>);

fn path() -> PathBuf {
    dirs::root().join("action-shortcuts.json")
}

impl Shortcuts {
    pub fn load() -> io::Result<Self> {
        match fs::read(path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn get(&self, target: &Target) -> Option<&str> {
        self.0
            .iter()
            .find(|binding| &binding.target == target)
            .map(|binding| binding.shortcut.as_str())
    }

    pub fn assign(
        &mut self,
        target: Target,
        title: &str,
        text: &str,
        reserved: &[(Hotkey, String)],
    ) -> Result<(), String> {
        let text = text.trim();
        let mut next = Self::load().map_err(|_| i18n::t("actions.shortcut_read"))?;
        if text.is_empty() {
            next.0.retain(|binding| binding.target != target);
        } else {
            let hotkey = Hotkey::parse(text).ok_or_else(|| i18n::t("actions.shortcut_invalid"))?;
            if let Some(key) = launcher_owner(&hotkey) {
                return Err(occupied(&hotkey, &format!("MyTools · {}", i18n::t(key))));
            }
            if !allowed(&hotkey) {
                return Err(i18n::t("actions.shortcut_invalid"));
            }
            let global = crate::config::load()
                .map(|config| config.hotkeys)
                .unwrap_or_default();
            for (binding, owner) in [
                (
                    Some(global.launcher_hotkey.as_str()),
                    "actions.shortcut_launcher",
                ),
                (global.selection(), "actions.shortcut_selection"),
            ] {
                if binding.and_then(Hotkey::parse).as_ref() == Some(&hotkey) {
                    return Err(occupied(&hotkey, &format!("MyTools · {}", i18n::t(owner))));
                }
            }
            if let Some((_, owner)) = reserved.iter().find(|(key, _)| key == &hotkey) {
                return Err(occupied(&hotkey, owner));
            }
            let shortcut = hotkey.to_string();
            if let Some(binding) = next.0.iter().find(|binding| {
                binding.target != target
                    && Hotkey::parse(&binding.shortcut).as_ref() == Some(&hotkey)
            }) {
                return Err(occupied(&hotkey, &binding.owner()));
            }
            next.0.retain(|binding| binding.target != target);
            next.0.push(Binding {
                target,
                shortcut,
                title: title.into(),
            });
        }

        let path = path();
        let bytes =
            serde_json::to_vec_pretty(&next).map_err(|_| i18n::t("actions.shortcut_write"))?;
        let save = || {
            fs::create_dir_all(path.parent().unwrap())?;
            let temporary = path.with_extension("tmp");
            fs::write(&temporary, bytes)?;
            fs::rename(&temporary, &path)
        };
        save().map_err(|_: io::Error| i18n::t("actions.shortcut_write"))?;
        *self = next;
        Ok(())
    }

    pub fn matches(
        &self,
        target: &Target,
        text: &str,
        modifiers: (bool, bool, bool, bool),
    ) -> bool {
        self.get(target)
            .and_then(Hotkey::parse)
            .filter(allowed)
            .is_some_and(|hotkey| {
                (hotkey.ctrl, hotkey.shift, hotkey.alt, hotkey.meta) == modifiers
                    && keys(&hotkey).is_some_and(|keys| keys.to_parts().last() == Some(text))
            })
    }
}

fn launcher_owner(hotkey: &Hotkey) -> Option<&'static str> {
    let plain = !hotkey.ctrl && !hotkey.alt && !hotkey.meta && !hotkey.shift;
    let ctrl_only = hotkey.ctrl && !hotkey.alt && !hotkey.meta && !hotkey.shift;
    match (hotkey.key.as_str(), plain, ctrl_only) {
        ("F1", true, _) => Some("actions.shortcut_license"),
        ("K", _, true) => Some("actions.title"),
        ("Enter", _, _) if hotkey.ctrl || hotkey.meta => Some("actions.execute"),
        ("Tab", _, _) => Some("actions.shortcut_tab"),
        ("Escape", _, _) => Some("actions.shortcut_dismiss"),
        ("Backspace" | "Delete", _, _) | ("A" | "C" | "V" | "X" | "Y" | "Z", _, true) => {
            Some("actions.shortcut_text")
        }
        _ => None,
    }
}

fn allowed(hotkey: &Hotkey) -> bool {
    let key = hotkey.key.as_str();
    if launcher_owner(hotkey).is_some() || matches!(key, "F1" | "Enter") {
        return false;
    }
    // A shift-only printable shortcut would intercept ordinary capitalized input.
    (hotkey.ctrl || hotkey.alt || hotkey.meta || key.starts_with('F') && key.len() > 1)
        && keys(hotkey).is_some()
}

fn keys(hotkey: &Hotkey) -> Option<slint::Keys> {
    let key = match hotkey.key.as_str() {
        "Left" => "LeftArrow".into(),
        "Right" => "RightArrow".into(),
        "Up" => "UpArrow".into(),
        "Down" => "DownArrow".into(),
        "Space" => " ".into(),
        key if key.len() == 1 => key.to_ascii_lowercase(),
        key => key.into(),
    };
    slint::Keys::from_parts([key.as_str()]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_and_plugin_shortcuts_match_only_their_configured_targets() {
        let builtin = Target::MyTools {
            action: ActionId::CopyPath,
        };
        let plugin = Target::Plugin {
            plugin: "clipboard".into(),
            command: "pin".into(),
            item: None,
        };
        let shortcuts = Shortcuts(vec![
            Binding {
                target: builtin.clone(),
                shortcut: "Ctrl+Shift+P".into(),
                title: String::new(),
            },
            Binding {
                target: plugin.clone(),
                shortcut: "F8".into(),
                title: "Pin".into(),
            },
        ]);
        let restored: Shortcuts =
            serde_json::from_slice(&serde_json::to_vec(&shortcuts).unwrap()).unwrap();
        assert!(restored.matches(&builtin, "p", (true, true, false, false)));
        assert!(!restored.matches(&plugin, "p", (true, true, false, false)));
        assert!(!restored.matches(&builtin, "p", (true, false, false, false)));
        let f8: slint::SharedString = slint::platform::Key::F8.into();
        assert!(restored.matches(&plugin, &f8, (false, false, false, false)));
        assert!(!allowed(&Hotkey::parse("Ctrl+K").unwrap()));
        assert!(!allowed(&Hotkey::parse("Shift+P").unwrap()));
        assert!(restored.0[0].owner().starts_with("MyTools · "));
        assert_eq!(restored.0[1].owner(), "clipboard · Pin");
    }
}
