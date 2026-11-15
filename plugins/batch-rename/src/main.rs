use mytools_plugin_api::{
    CommandArgument, PluginActionRequest, PluginEffect, Resource, SearchResponse, i18n::Catalog,
    paths, text,
};
use std::{cell::RefCell, path::PathBuf};

mod rename;
mod rules;

#[derive(Default)]
struct Session {
    selected: Vec<PathBuf>,
    preview: Option<(u64, Vec<rename::Change>)>,
    generation: u64,
    completed: Option<String>,
}

impl Session {
    fn search(&mut self, query: &str, catalog: &Catalog) -> SearchResponse {
        self.preview = None;
        let item = if let Some(summary) = &self.completed {
            text::editor_result(catalog.get("name"), summary.clone(), summary.clone())
        } else if self.selected.is_empty() {
            text::editor_prompt(catalog.get("name"), catalog.get("selection.none"))
        } else {
            match rules::Rules::parse(query, catalog)
                .and_then(|rules| rename::plan(&self.selected, &rules, catalog))
            {
                Ok(changes) if changes.is_empty() => {
                    let names = self
                        .selected
                        .iter()
                        .map(|path| paths::display(path))
                        .collect::<Vec<_>>()
                        .join("\n");
                    preview_text(
                        catalog,
                        catalog.format("selection.current", &[("names", &names)]),
                    )
                }
                Ok(changes) => {
                    let mut item =
                        preview_text(catalog, summary("preview.text", &changes, catalog));
                    self.generation = self.generation.wrapping_add(1);
                    item.actions = Some(vec![
                        text::command("apply", catalog.get("preview.apply").to_owned())
                            .with_item(self.generation.to_string()),
                    ]);
                    self.preview = Some((self.generation, changes));
                    item
                }
                Err(error) => text::editor_prompt(catalog.get("rename.error"), error),
            }
        };
        SearchResponse {
            items: vec![item],
            ..Default::default()
        }
    }

    fn execute(
        &mut self,
        request: PluginActionRequest,
        catalog: &Catalog,
    ) -> Result<PluginEffect, String> {
        match request.command.as_str() {
            "rename-files" => {
                let Some(CommandArgument::Files { paths }) = request.argument else {
                    return Err(catalog.get("selection.none").into());
                };
                if paths.is_empty() {
                    return Err(catalog.get("selection.none").into());
                }
                self.selected = paths;
                self.preview = None;
                self.completed = None;
                Ok(PluginEffect {
                    search_query: Some(catalog.get("rules.template").into()),
                    ..Default::default()
                })
            }
            "apply" => {
                let generation = request
                    .item
                    .as_deref()
                    .and_then(|id| id.parse::<u64>().ok());
                if !self
                    .preview
                    .as_ref()
                    .is_some_and(|(id, _)| Some(*id) == generation)
                {
                    return Err(catalog.get("rename.changed").into());
                }
                let (_, changes) = self.preview.take().expect("the preview ID was checked");
                rename::apply(&changes, catalog)?;
                self.completed = Some(summary("rename.done", &changes, catalog));
                self.selected.clear();
                Ok(PluginEffect {
                    refresh: true,
                    ..Default::default()
                })
            }
            _ => Err(catalog.get("command.invalid").into()),
        }
    }
}

fn preview_text(catalog: &Catalog, body: String) -> mytools_plugin_api::SearchItem {
    let mut item = text::editor_result(catalog.get("preview.title"), String::new(), body);
    item.resource = Resource::PluginItem {
        id: "preview".into(),
    };
    item.actions = Some(vec![]);
    item
}

fn summary(key: &str, changes: &[rename::Change], catalog: &Catalog) -> String {
    let changes_text = changes
        .iter()
        .map(|change| {
            format!(
                "{} → {}",
                paths::display(&change.from),
                change.to.file_name().unwrap_or_default().to_string_lossy()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    catalog.format(
        key,
        &[
            ("count", &changes.len().to_string()),
            ("changes", &changes_text),
        ],
    )
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .expect("bundled batch rename locales");
    let session = RefCell::new(Session::default());
    mytools_plugin_api::serve_requests(
        |request| {
            if request.scoped {
                session.borrow_mut().search(&request.query, &catalog)
            } else {
                SearchResponse::default()
            }
        },
        |request| match session.borrow_mut().execute(request, &catalog) {
            Ok(effect) => SearchResponse {
                effect: Some(effect),
                ..Default::default()
            },
            Err(error) => SearchResponse {
                error: Some(error),
                ..Default::default()
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn request(command: &str) -> PluginActionRequest {
        PluginActionRequest {
            command: command.into(),
            item: None,
            editable: None,
            argument: None,
        }
    }

    fn catalog() -> Catalog {
        Catalog::from_json(include_str!("../locales/en.json"), "{}").unwrap()
    }

    fn open(session: &mut Session, paths: Vec<PathBuf>, catalog: &Catalog) {
        let mut request = request("rename-files");
        request.argument = Some(CommandArgument::Files { paths });
        assert!(
            session
                .execute(request, catalog)
                .unwrap()
                .search_query
                .is_some()
        );
    }

    fn preview(session: &mut Session, rules: &str, catalog: &Catalog) -> String {
        let response = session.search(rules, catalog);
        response.items[0].actions.as_ref().unwrap()[0]
            .item
            .clone()
            .unwrap()
    }

    #[test]
    fn confirmation_only_applies_the_latest_preview_and_preserves_extensions() {
        let catalog = catalog();
        let room = tempfile::tempdir().unwrap();
        let source = room.path().join("截图.png");
        let other = room.path().join("notes.txt");
        fs::write(&source, "image").unwrap();
        fs::write(&other, "notes").unwrap();
        let mut session = Session::default();
        open(&mut session, vec![source.clone(), other.clone()], &catalog);
        let first = preview(&mut session, "prefix=old-", &catalog);
        let second = preview(&mut session, "name=Archive\nnumber=3\nwidth=2", &catalog);
        assert!(source.exists() && other.exists());

        let mut apply = request("apply");
        apply.item = Some(first);
        assert!(session.execute(apply.clone(), &catalog).is_err());
        apply.item = Some(second);
        assert!(session.execute(apply.clone(), &catalog).unwrap().refresh);
        assert!(session.execute(apply, &catalog).is_err());
        assert_eq!(
            fs::read_to_string(room.path().join("Archive 03.png")).unwrap(),
            "image"
        );
        assert_eq!(
            fs::read_to_string(room.path().join("Archive 04.txt")).unwrap(),
            "notes"
        );
        assert!(!source.exists() && !other.exists());
    }

    #[test]
    fn rechecks_collisions_created_after_preview() {
        let catalog = catalog();
        let room = tempfile::tempdir().unwrap();
        let source = room.path().join("report.txt");
        let target = room.path().join("old-report.txt");
        fs::write(&source, "original").unwrap();
        let mut session = Session::default();
        open(&mut session, vec![source.clone()], &catalog);

        let mut apply = request("apply");
        apply.item = Some(preview(&mut session, "prefix=old-", &catalog));
        fs::write(&target, "new occupant").unwrap();
        assert!(
            session
                .execute(apply, &catalog)
                .unwrap_err()
                .contains("old-report.txt")
        );
        assert_eq!(fs::read_to_string(source).unwrap(), "original");
        assert_eq!(fs::read_to_string(target).unwrap(), "new occupant");
    }

    #[test]
    fn rejects_invalid_names_and_duplicate_destinations() {
        let catalog = catalog();
        let room = tempfile::tempdir().unwrap();
        let first = room.path().join("one.txt");
        let second = room.path().join("two.txt");
        fs::write(&first, "").unwrap();
        fs::write(&second, "").unwrap();
        let mut session = Session::default();
        open(&mut session, vec![first, second], &catalog);
        for rules in ["name=CON", "name=../other", "name=same"] {
            let results = session.search(rules, &catalog);
            assert!(
                results.items[0].actions.as_ref().unwrap().is_empty(),
                "{rules}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn changes_only_the_case_of_a_file_name() {
        let catalog = catalog();
        let room = tempfile::tempdir().unwrap();
        let source = room.path().join("readme.txt");
        fs::write(&source, "original").unwrap();
        let rules = rules::Rules::parse("name=README", &catalog).unwrap();
        let changes = rename::plan(&[source], &rules, &catalog).unwrap();
        rename::apply(&changes, &catalog).unwrap();
        assert_eq!(
            fs::read_dir(room.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .file_name(),
            "README.txt"
        );
    }

    #[cfg(windows)]
    #[test]
    fn rolls_back_when_one_file_cannot_be_renamed() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let catalog = catalog();
        let room = tempfile::tempdir().unwrap();
        let first = room.path().join("first.txt");
        let locked = room.path().join("locked.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&locked, "locked").unwrap();
        let rules = rules::Rules::parse("prefix=new-", &catalog).unwrap();
        let changes = rename::plan(&[first.clone(), locked.clone()], &rules, &catalog).unwrap();
        // Allow reads and writes, but omit FILE_SHARE_DELETE so the second move fails.
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&locked)
            .unwrap();
        let error = rename::apply(&changes, &catalog).unwrap_err();
        drop(lock);
        assert!(error.contains("locked.txt"));
        assert_eq!(fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(fs::read_to_string(&locked).unwrap(), "locked");
        assert!(!room.path().join("new-first.txt").exists());
    }
}
