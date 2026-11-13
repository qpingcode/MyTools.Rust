use crate::i18n;
use crate::platform;
use anyhow::{Result, bail, ensure};
use mytools_plugin_api::{ActionId, ActionSpec, Resource};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Action {
    pub id: ActionId,
    pub title: Option<String>,
    pub title_key: &'static str,
    pub icon: String,
    operation: Operation,
    pub(crate) selection: Option<crate::history::Selection>,
    input: Input,
}

/// What the host gathers before running a plugin command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Input {
    #[default]
    None,
    /// The text selected in the application in front becomes the action's `item`.
    Selection {
        /// The plugin's scope ("alias "), where a result for a read-only selection is shown.
        scope: Option<String>,
    },
}

#[derive(Clone, Debug)]
enum Operation {
    Open(PathBuf),
    OpenApplication(PathBuf),
    OpenUrl(String),
    Copy(String),
    Reveal(PathBuf),
    Launch(PathBuf),
    RunAsAdmin(PathBuf),
    Plugin {
        command: String,
        /// Sent only when the action declared one; plugins identify their own targets.
        item: Option<String>,
        process: Option<std::sync::Arc<crate::search_process::SearchProcess>>,
    },
    /// Runs the current query again with a longer timeout.
    Retry(u32),
}

pub enum ActionEffect {
    CopyText(String),
    Completed,
    Plugin(mytools_plugin_api::PluginEffect),
    /// Search the current query again as this retry attempt.
    Retry(u32),
}

impl Action {
    /// Call only after all effects (including clipboard/paste) have succeeded.
    /// Only primary actions of nonempty global searches carry a selection.
    pub fn record_success(&self) -> Result<()> {
        if let Some(selection) = &self.selection {
            selection.record()?;
        }
        Ok(())
    }

    pub fn dismiss_on_success(&self) -> bool {
        matches!(
            self.operation,
            Operation::OpenApplication(_)
                | Operation::Launch(_)
                | Operation::RunAsAdmin(_)
                | Operation::OpenUrl(_)
        )
    }

    /// Whether the host must read the selected text and pass it with [`Self::with_item`].
    pub fn needs_selection(&self) -> bool {
        matches!(self.input, Input::Selection { .. })
    }

    /// For a selection command: the query that opens its plugin's scope, if it has one.
    pub fn selection_scope(&self) -> Option<&str> {
        match &self.input {
            Input::Selection { scope } => scope.as_deref(),
            Input::None => None,
        }
    }

    /// This plugin command with `item`, e.g. the selected text.
    pub fn with_item(&self, item: String) -> Self {
        let mut action = self.clone();
        if let Operation::Plugin { item: target, .. } = &mut action.operation {
            *target = Some(item);
        }
        action.input = Input::None;
        action
    }

    /// OS operations run on a worker; clipboard effects are applied by the UI thread.
    pub fn execute(&self) -> Result<ActionEffect> {
        match &self.operation {
            Operation::Copy(text) => return Ok(ActionEffect::CopyText(text.clone())),
            Operation::Retry(attempt) => return Ok(ActionEffect::Retry(*attempt)),
            Operation::Open(path) | Operation::OpenApplication(path) => {
                platform::open_application(path)?
            }
            Operation::OpenUrl(url) => platform::open_url(url)?,
            Operation::Reveal(path) => platform::reveal(path)?,
            Operation::RunAsAdmin(path) => platform::run_as_admin(path)?,
            Operation::Plugin {
                command,
                item,
                process,
            } => {
                let process = process
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!(i18n::t("error.process_stopped")))?;
                return Ok(ActionEffect::Plugin(
                    process.execute(command, item.as_deref())?,
                ));
            }
            Operation::Launch(path) => {
                std::process::Command::new(path)
                    .current_dir(path.parent().unwrap_or(Path::new(".")))
                    .env("MYTOOLS_LOCALE", mytools_plugin_api::i18n::language())
                    .env(
                        mytools_plugin_api::crash::ENV,
                        mytools_plugin_api::crash::directory(),
                    )
                    .spawn()?;
            }
        }
        Ok(ActionEffect::Completed)
    }

    /// Retries a search whose plugin did not answer in time.
    pub(crate) fn retry(attempt: u32) -> Self {
        Self {
            id: ActionId::Plugin,
            title: Some(i18n::t("actions.retry")),
            title_key: "actions.execute",
            icon: "builtin:refresh-cw".into(),
            operation: Operation::Retry(attempt),
            selection: None,
            input: Input::None,
        }
    }

    /// A command another plugin contributes to this result; `item` is the result's main value.
    pub(crate) fn contributed(
        action: &crate::plugins::ContributedAction,
        item: String,
        process: &std::sync::Arc<crate::search_process::SearchProcess>,
    ) -> Self {
        Self {
            id: ActionId::Plugin,
            title: Some(action.title.clone()),
            title_key: "actions.execute",
            icon: action.icon.clone(),
            operation: Operation::Plugin {
                command: action.command.clone(),
                item: Some(item),
                process: Some(process.clone()),
            },
            selection: None,
            input: Input::None,
        }
    }

    /// Runs a command the plugin declared in its manifest. It carries no `item` unless it
    /// works on the selection (`selection` is then `Some(scope)`), which the host adds.
    pub(crate) fn command(
        command: &str,
        title: &str,
        icon: &mytools_plugin_api::Icon,
        process: &std::sync::Arc<crate::search_process::SearchProcess>,
        selection: Option<Option<String>>,
    ) -> Self {
        let icon = match icon {
            mytools_plugin_api::Icon::Name(name) if name.starts_with("builtin:") => name.clone(),
            _ => "builtin:play".into(),
        };
        Self {
            id: ActionId::Plugin,
            title: Some(title.into()),
            title_key: "actions.execute",
            icon,
            operation: Operation::Plugin {
                command: command.into(),
                item: None,
                process: Some(process.clone()),
            },
            selection: None,
            input: match selection {
                Some(scope) => Input::Selection { scope },
                None => Input::None,
            },
        }
    }

    pub(crate) fn bind_plugin(
        &mut self,
        process: &std::sync::Arc<crate::search_process::SearchProcess>,
    ) {
        if let Operation::Plugin {
            process: target, ..
        } = &mut self.operation
        {
            *target = Some(process.clone());
        }
    }
}

pub fn resolve_actions(
    resource: &Resource,
    specified: Option<&[ActionSpec]>,
) -> Result<Vec<Action>> {
    validate_resource(resource)?;
    let defaults;
    let specs = match specified {
        Some(specs) => specs,
        None => {
            defaults = default_ids(resource)
                .into_iter()
                .map(ActionSpec::from)
                .collect::<Vec<_>>();
            &defaults
        }
    };
    let mut seen = HashSet::new();
    let mut actions = Vec::new();
    for spec in specs {
        if !seen.insert((spec.id, spec.command.clone())) {
            continue;
        }
        // Plugin commands work on any result type; the result's own resource stays what the
        // host and other plugins see, and the action carries its own `item`.
        let (title_key, operation) = if spec.id == ActionId::Plugin {
            let command = spec
                .command
                .as_ref()
                .filter(|command| !command.is_empty())
                .ok_or_else(|| anyhow::anyhow!(i18n::t("error.response_invalid")))?;
            (
                "actions.execute",
                Operation::Plugin {
                    command: command.clone(),
                    item: spec.item.clone(),
                    process: None,
                },
            )
        } else {
            resolve(resource, spec.id)?
        };
        let icon = match operation {
            Operation::Copy(_) => "builtin:copy",
            Operation::Reveal(_) => "builtin:folder-open",
            Operation::RunAsAdmin(_) => "builtin:shield",
            Operation::Plugin { ref command, .. } => match command.as_str() {
                "delete" | "clear" => "builtin:trash",
                "copy" => "builtin:copy",
                _ => "builtin:clipboard",
            },
            _ => "builtin:external-link",
        };
        actions.push(Action {
            id: spec.id,
            title: spec
                .title
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .map(str::to_owned),
            title_key,
            icon: icon.into(),
            operation,
            selection: None,
            input: Input::None,
        });
    }
    Ok(actions)
}

fn default_ids(resource: &Resource) -> Vec<ActionId> {
    use ActionId::*;
    match resource {
        Resource::Url { .. } => vec![OpenInBrowser, CopyText],
        Resource::Text { .. } => vec![CopyText],
        Resource::PluginItem { .. } => vec![],
        Resource::Application { path } if mytools_plugin_api::paths::is_packaged_app(path) => {
            vec![Open]
        }
        Resource::Application { path } => {
            let mut ids = vec![Open];
            if platform::can_run_as_admin(path) {
                ids.push(RunAsAdmin);
            }
            ids.extend([CopyPath, Reveal]);
            ids
        }
        Resource::File { path } | Resource::Folder { path } => {
            let mut ids = vec![Open, CopyPath];
            if path.file_name().is_some() {
                ids.push(CopyName);
            }
            ids.push(Reveal);
            ids
        }
    }
}

fn resource_path(resource: &Resource) -> Option<&Path> {
    match resource {
        // A packaged app's target is not a file path to validate, copy or reveal.
        Resource::Application { path } if mytools_plugin_api::paths::is_packaged_app(path) => None,
        Resource::File { path } | Resource::Folder { path } | Resource::Application { path } => {
            Some(path)
        }
        _ => None,
    }
}

fn validate_resource(resource: &Resource) -> Result<()> {
    if let Some(path) = resource_path(resource) {
        ensure!(path.is_absolute(), i18n::t("error.absolute_path"));
        ensure!(
            !path.as_os_str().as_encoded_bytes().contains(&0),
            i18n::t("error.path_null")
        );
    }
    if let Resource::Url { url } = resource {
        ensure!(!url.contains('\0'), i18n::t("error.url_null"));
        let parsed = url::Url::parse(url)?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
            i18n::t("error.url_scheme")
        );
    }
    Ok(())
}

fn resolve(resource: &Resource, id: ActionId) -> Result<(&'static str, Operation)> {
    use ActionId::*;
    match (id, resource) {
        (CopyText, Resource::Text { text }) => Ok(("actions.copy", Operation::Copy(text.clone()))),
        (CopyText, Resource::Url { url }) => {
            Ok(("actions.copy_link", Operation::Copy(url.clone())))
        }
        (OpenInBrowser, Resource::Url { url }) => {
            Ok(("actions.open_browser", Operation::OpenUrl(url.clone())))
        }
        (Open, Resource::File { path }) => Ok(("actions.open", Operation::Open(path.clone()))),
        (Open, Resource::Folder { path }) => {
            Ok(("actions.open_folder", Operation::Open(path.clone())))
        }
        (Open, Resource::Application { path }) => {
            Ok(("actions.launch", Operation::OpenApplication(path.clone())))
        }
        (Launch, Resource::Application { path }) => {
            Ok(("actions.launch", Operation::Launch(path.clone())))
        }
        (RunAsAdmin, Resource::Application { path }) if platform::can_run_as_admin(path) => {
            Ok(("actions.run_as_admin", Operation::RunAsAdmin(path.clone())))
        }
        (CopyPath | Reveal, resource) if resource_path(resource).is_some() => {
            let path = resource_path(resource).unwrap();
            let shortcut = matches!(resource, Resource::Application { .. })
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
            if id == CopyPath {
                Ok((
                    if shortcut {
                        "actions.copy_shortcut_path"
                    } else {
                        "actions.copy_path"
                    },
                    Operation::Copy(mytools_plugin_api::paths::display(path)),
                ))
            } else {
                Ok((
                    if shortcut {
                        "actions.reveal_shortcut"
                    } else {
                        "actions.reveal"
                    },
                    Operation::Reveal(path.into()),
                ))
            }
        }
        (CopyName, Resource::File { path } | Resource::Folder { path }) => {
            let name = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!(i18n::t("error.file_name")))?;
            Ok((
                if matches!(resource, Resource::Folder { .. }) {
                    "actions.copy_folder_name"
                } else {
                    "actions.copy_name"
                },
                Operation::Copy(name.to_string_lossy().into_owned()),
            ))
        }
        _ => bail!(i18n::format(
            "error.incompatible",
            &[("id", &format!("{id:?}"))]
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_commands_preserve_distinct_actions_and_deduplicate_identical_commands() {
        let specs = ["paste", "copy", "delete", "clear", "copy"].map(|command| ActionSpec {
            id: ActionId::Plugin,
            title: Some(command.into()),
            command: Some(command.into()),
            item: Some("entry-1".into()),
        });
        let resource = Resource::PluginItem {
            id: "entry-1".into(),
        };
        let actions = resolve_actions(&resource, Some(&specs)).unwrap();
        assert_eq!(actions.len(), 4);
        assert_eq!(actions[0].title.as_deref(), Some("paste"));
        assert_eq!(actions[2].icon, "builtin:trash");
        assert!(resolve_actions(&resource, Some(&[ActionId::Plugin.into()])).is_err());
    }

    #[test]
    fn plugin_actions_send_only_their_declared_item_on_any_resource() {
        let declared = mytools_plugin_api::text::command("paste", "Paste".into()).with_item("e1");
        let bare = mytools_plugin_api::text::command("clear", "Clear".into());
        let actions = resolve_actions(&file(), Some(&[declared, bare])).unwrap();
        let items: Vec<_> = actions
            .iter()
            .map(|action| match &action.operation {
                Operation::Plugin { item, .. } => item.clone(),
                _ => panic!("expected a plugin operation"),
            })
            .collect();
        // No fallback to the resource: a file result does not leak its path, and an
        // undeclared item is simply absent.
        assert_eq!(items, [Some("e1".to_owned()), None]);
    }

    fn file() -> Resource {
        Resource::File {
            path: std::env::temp_dir().join("报告, final.pdf"),
        }
    }

    #[test]
    fn generates_defaults_and_correct_copy_payloads() {
        let actions = resolve_actions(&file(), None).unwrap();
        assert!(!actions[0].dismiss_on_success());
        assert_eq!(
            actions.iter().map(|action| action.id).collect::<Vec<_>>(),
            vec![
                ActionId::Open,
                ActionId::CopyPath,
                ActionId::CopyName,
                ActionId::Reveal
            ]
        );
        assert!(
            matches!(actions[2].execute().unwrap(), ActionEffect::CopyText(text) if text == "报告, final.pdf")
        );
        let url = Resource::Url {
            url: "https://example.com/a?q=1".into(),
        };
        let actions = resolve_actions(&url, None).unwrap();
        assert_eq!(actions[0].id, ActionId::OpenInBrowser);
        assert!(actions[0].dismiss_on_success());
        assert!(!actions[1].dismiss_on_success());
        let application = Resource::Application {
            path: std::env::temp_dir().join("example.exe"),
        };
        for id in [ActionId::Open, ActionId::Launch] {
            assert!(
                resolve_actions(&application, Some(&[id.into()])).unwrap()[0].dismiss_on_success()
            );
        }
        assert!(
            matches!(actions[1].execute().unwrap(), ActionEffect::CopyText(text) if text == "https://example.com/a?q=1")
        );
        assert_eq!(
            resolve_actions(&Resource::Text { text: "2".into() }, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            resolve_actions(
                &Resource::Folder {
                    path: std::env::temp_dir()
                },
                None
            )
            .unwrap()[0]
                .title_key,
            "actions.open_folder"
        );
    }

    #[cfg(windows)]
    #[test]
    fn copied_paths_use_ordinary_drive_and_unc_syntax() {
        for (source, expected) in [
            (r"\\?\C:\应用程序\App.lnk", r"C:\应用程序\App.lnk"),
            (r"\\?\UNC\server\share\文件.txt", r"\\server\share\文件.txt"),
            (r"C:\ordinary\file.txt", r"C:\ordinary\file.txt"),
        ] {
            let path = PathBuf::from(source);
            for resource in [
                Resource::Application { path: path.clone() },
                Resource::File { path: path.clone() },
                Resource::Folder { path: path.clone() },
            ] {
                let actions =
                    resolve_actions(&resource, Some(&[ActionId::CopyPath.into()])).unwrap();
                assert!(
                    matches!(actions[0].execute().unwrap(), ActionEffect::CopyText(text) if text == expected)
                );
                assert_eq!(resource_path(&resource), Some(path.as_path()));
            }
        }
    }

    #[test]
    fn explicit_actions_replace_defaults_preserve_order_and_deduplicate() {
        let specs = vec![
            ActionSpec {
                id: ActionId::CopyName,
                title: Some("复制报告名称".into()),
                command: None,
                item: None,
            },
            ActionId::Open.into(),
            ActionId::CopyName.into(),
        ];
        let actions = resolve_actions(&file(), Some(&specs)).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].title.as_deref(), Some("复制报告名称"));
        assert_eq!(actions[1].id, ActionId::Open);
        assert!(resolve_actions(&file(), Some(&[])).unwrap().is_empty());
    }

    #[test]
    fn rejects_mismatched_actions_and_invalid_resources() {
        assert!(resolve_actions(&file(), Some(&[ActionId::OpenInBrowser.into()])).is_err());
        assert!(
            resolve_actions(
                &Resource::Url {
                    url: "javascript:alert(1)".into()
                },
                None
            )
            .is_err()
        );
        assert!(
            resolve_actions(
                &Resource::File {
                    path: "relative.txt".into()
                },
                None
            )
            .is_err()
        );
        assert!(
            resolve_actions(
                &Resource::File {
                    path: std::env::temp_dir().join("bad\0path")
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn application_shortcuts_have_unambiguous_labels() {
        let actions = resolve_actions(
            &Resource::Application {
                path: std::env::temp_dir().join("Rider.lnk"),
            },
            None,
        )
        .unwrap();
        assert_eq!(actions[0].title_key, "actions.launch");
        assert_eq!(
            actions
                .iter()
                .find(|action| action.id == ActionId::CopyPath)
                .unwrap()
                .title_key,
            "actions.copy_shortcut_path"
        );
        assert_eq!(
            actions
                .iter()
                .find(|action| action.id == ActionId::Reveal)
                .unwrap()
                .title_key,
            "actions.reveal_shortcut"
        );
    }

    #[test]
    fn administrator_action_is_limited_to_windows_application_executables() {
        for extension in ["exe", "com", "lnk", "url", "app"] {
            let resource = Resource::Application {
                path: std::env::temp_dir().join(format!("app.{extension}")),
            };
            let expected = cfg!(windows) && matches!(extension, "exe" | "com" | "lnk");
            let actions = resolve_actions(&resource, None).unwrap();
            assert_eq!(
                actions
                    .iter()
                    .any(|action| action.id == ActionId::RunAsAdmin),
                expected
            );
            let explicit = resolve_actions(&resource, Some(&[ActionId::RunAsAdmin.into()]));
            assert_eq!(explicit.is_ok(), expected);
            if let Ok(actions) = explicit {
                assert_eq!(actions[0].icon, "builtin:shield");
                assert_eq!(actions[0].title_key, "actions.run_as_admin");
                assert!(actions[0].dismiss_on_success());
            }
        }
        assert!(resolve_actions(&file(), Some(&[ActionId::RunAsAdmin.into()])).is_err());
    }
}
