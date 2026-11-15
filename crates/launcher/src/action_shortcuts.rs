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
        text: &str,
        reserved: &[Hotkey],
    ) -> Result<(), &'static str> {
        let text = text.trim();
        let mut next = Self::load().map_err(|_| "actions.shortcut_read")?;
        if text.is_empty() {
            next.0.retain(|binding| binding.target != target);
        } else {
            let hotkey = Hotkey::parse(text)
                .filter(allowed)
                .ok_or("actions.shortcut_invalid")?;
            let global = crate::config::load()
                .map(|config| config.hotkeys)
                .unwrap_or_default();
            if reserved.contains(&hotkey)
                || [Some(global.launcher_hotkey.as_str()), global.selection()]
                    .into_iter()
                    .flatten()
                    .filter_map(Hotkey::parse)
                    .any(|global| global == hotkey)
            {
                return Err("actions.shortcut_global");
            }
            let shortcut = hotkey.to_string();
            if next
                .0
                .iter()
                .any(|binding| binding.target != target && binding.shortcut == shortcut)
            {
                return Err("actions.shortcut_duplicate");
            }
            next.0.retain(|binding| binding.target != target);
            next.0.push(Binding { target, shortcut });
        }

        let path = path();
        let bytes = serde_json::to_vec_pretty(&next).map_err(|_| "actions.shortcut_write")?;
        let save = || {
            fs::create_dir_all(path.parent().unwrap())?;
            let temporary = path.with_extension("tmp");
            fs::write(&temporary, bytes)?;
            fs::rename(&temporary, &path)
        };
        save().map_err(|_: io::Error| "actions.shortcut_write")?;
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

fn allowed(hotkey: &Hotkey) -> bool {
    let key = hotkey.key.as_str();
    if key == "F1"
        || matches!(key, "Escape" | "Enter" | "Tab" | "Backspace" | "Delete")
        || (hotkey.ctrl
            && !hotkey.alt
            && !hotkey.meta
            && !hotkey.shift
            && matches!(key, "A" | "C" | "K" | "V" | "X" | "Y" | "Z"))
    {
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
            },
            Binding {
                target: plugin.clone(),
                shortcut: "F8".into(),
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
    }
}
