use crate::{
    AboutWindow, AppTray, I18n, MainWindow, ResultData, TextFit, Theme, actions, commands, config,
    desktop, i18n, icons,
    search::{Event, Query, SearchWorker},
    theme, tray,
};
use mytools_core::{Action, Entry, Notice, SearchResults};
use slint::{ComponentHandle, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    displayed_query: String,
    pub entries: Vec<Entry>,
    pub panel: Option<Vec<Action>>,
    desktop_error: Option<String>,
    /// Plugin or command shortcuts that could not be registered.
    hotkey_error: Option<String>,
}

pub fn run(instance: &crate::single_instance::Instance) -> anyhow::Result<()> {
    let config = config::load();
    let load_options = config
        .as_ref()
        .map(config::Config::load_options)
        .unwrap_or_default();
    let requested = config
        .as_ref()
        .map(|config| config.language.as_str())
        .unwrap_or("auto");
    mytools_plugin_api::i18n::initialize(requested);
    if let Ok(config) = &config {
        config::migrate_plugin_shortcuts(config);
    }
    let theme_errors = theme::configure(
        config
            .as_ref()
            .map(|config| config.theme.clone())
            .unwrap_or_default(),
    );
    desktop::configure_backend()?;
    let ui = MainWindow::new()?;
    theme::apply_configured(&ui.global::<Theme>());
    let geometry_tracker = crate::window_geometry::Tracker::new(&ui);
    let tray = AppTray::new()?;
    let about = AboutWindow::new()?;
    theme::apply_configured(&about.global::<Theme>());
    ui.global::<I18n>().on_text(|key| i18n::t(&key).into());
    ui.global::<TextFit>()
        .on_path(|text, width, font_size| crate::elide::fit_path(&text, width, font_size).into());
    ui.global::<TextFit>()
        .on_highlight(|text, query, color, width, font_size| {
            crate::highlight::styled(&text, &query, color, width, font_size)
        });
    tray.global::<I18n>().on_text(|key| i18n::t(&key).into());
    about.global::<I18n>().on_text(|key| i18n::t(&key).into());
    tray.set_tooltip_text(
        i18n::format(
            "tray.tooltip",
            &[(
                "shortcut",
                if cfg!(target_os = "macos") {
                    "Option+Space"
                } else {
                    "Alt+Space"
                },
            )],
        )
        .into(),
    );
    let font = if cfg!(windows) {
        "Microsoft YaHei UI"
    } else {
        "PingFang SC"
    };
    ui.set_app_font_family(font.into());
    ui.set_shortcut_label(
        if cfg!(target_os = "macos") {
            "⌘K"
        } else {
            "Ctrl+K"
        }
        .into(),
    );
    about.set_app_font_family(font.into());
    let notifier = crate::notices::Notifier::new(font);
    // Startup problems are shown once the event loop runs, next to plugin notices.
    let mut startup_notices = Vec::new();
    let mut initial = State::default();
    if let Err(error) = &config {
        mytools_plugin_api::log::error(
            "config.invalid",
            format!("config.json is invalid: {error:#}"),
        );
    }
    for error in &theme_errors {
        mytools_plugin_api::log::error("config.invalid", format!("theme setting ignored: {error}"));
    }
    if let Err(error) = config {
        initial.desktop_error = Some(i18n::format(
            "config.invalid",
            &[("error", &error.to_string())],
        ));
    } else if !theme_errors.is_empty() {
        initial.desktop_error = Some(i18n::format(
            "config.invalid",
            &[("error", &theme_errors.join("; "))],
        ));
    }
    if let Some(error) = &initial.desktop_error {
        startup_notices.push(Notice {
            title: i18n::t("notice.config"),
            message: error.clone(),
        });
    }
    let state = Rc::new(RefCell::new(initial));
    let icons = icons::Loader::new(&ui)?;
    let (worker, responses) = SearchWorker::start(load_options);
    let desktop: Rc<RefCell<Option<desktop::Desktop>>> = Rc::default();
    wire_search(&ui, &state, &worker);
    actions::wire(&ui, &state);
    let tray_controller = tray::Controller::new(&tray, &ui)?;
    let weak_about = about.as_weak();
    ui.on_show_license(move || {
        if let Some(about) = weak_about.upgrade() {
            let _ = about.show();
        }
    });

    let weak = ui.as_weak();
    let response_state = state.clone();
    let response_notifier = notifier.clone();
    let response_desktop = desktop.clone();
    let _responses = slint::spawn_local(async move {
        while let Ok(event) = responses.recv().await {
            let Some(ui) = weak.upgrade() else { break };
            let mut state = response_state.borrow_mut();
            match event {
                // The loading delay counts from here, not from the keystroke.
                Event::Started { generation } if generation == state.generation => {
                    ui.set_search_running(true);
                }
                Event::Completed(completed) if completed.generation == state.generation => {
                    apply_results(&ui, &mut state, completed.results, &icons);
                }
                Event::Notices(notices) => response_notifier.show(notices),
                Event::Hotkeys(bindings) => {
                    let mut desktop = response_desktop.borrow_mut();
                    // Without the manager there is nothing to register.
                    let Some(desktop) = desktop.as_mut() else {
                        continue;
                    };
                    let errors = desktop.bind(bindings);
                    state.hotkey_error = (!errors.is_empty()).then(|| errors.join("；"));
                    response_notifier.show(
                        errors
                            .into_iter()
                            .map(|message| Notice {
                                title: i18n::t("notice.hotkey"),
                                message,
                            })
                            .collect(),
                    );
                }
                _ => {} // Superseded by newer input.
            }
        }
    })?;

    let (command_sender, command_receiver) = async_channel::unbounded();
    let command_notifier = notifier.clone();
    let _commands = slint::spawn_local(async move {
        while let Ok(action) = command_receiver.recv().await {
            commands::run(action, &command_notifier);
        }
    })?;
    let run_command = move |action| {
        let _ = command_sender.send_blocking(action);
    };
    match desktop::Desktop::new(&ui, run_command) {
        Ok(created) => *desktop.borrow_mut() = Some(created),
        Err(error) => {
            // Without Alt+Space the window would be hard to reach, so it opens as well.
            let message = format!("{error:#}");
            state.borrow_mut().desktop_error = Some(message.clone());
            startup_notices.push(Notice {
                title: i18n::t("notice.hotkey"),
                message,
            });
            desktop::show(&ui);
        }
    }
    let startup_notifier = notifier.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
        startup_notifier.show(startup_notices);
    });
    ui.invoke_search("".into());
    // Starting MyTools again while it runs shows this window instead.
    let weak = ui.as_weak();
    instance.on_show(move || {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                desktop::show(&ui);
            }
        });
    });
    if std::env::args().any(|argument| argument == "--show") {
        desktop::show(&ui);
    }
    tray.show()?;
    // Hiding the launcher must never terminate the resident application.
    let result = slint::run_event_loop_until_quit();
    drop(desktop.borrow_mut().take());
    drop(worker);
    result?;
    let restart = tray_controller.restart_requested();
    drop(tray_controller);
    drop(tray);
    drop(about);
    drop(geometry_tracker);
    drop(ui);
    if restart {
        tray::restart()?;
    }
    Ok(())
}

fn wire_search(ui: &MainWindow, state: &Rc<RefCell<State>>, worker: &SearchWorker) {
    let requests = worker.sender();
    let weak = ui.as_weak();
    let state = state.clone();
    ui.on_search(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state.borrow_mut();
        actions::close_panel(&ui, &mut state);
        state.generation += 1;
        let searching = match requests.plugin_for(&text) {
            Some(name) => i18n::format("search.loading_plugin", &[("name", &name)]),
            None => i18n::t("search.loading"),
        };
        ui.set_searching_status(searching.into());
        ui.set_searching(true);
        // Typing never starts the indicator, but one already showing stays until results.
        if !ui.get_show_loading() {
            ui.set_search_running(false);
        }
        requests.send(Query {
            generation: state.generation,
            text: text.into(),
            attempt: crate::search::take_retry_attempt(),
        });
    });
}

fn apply_results(
    ui: &MainWindow,
    state: &mut State,
    results: SearchResults,
    icons: &icons::Loader,
) {
    ui.set_detail_view(results.view == mytools_plugin_api::View::ListDetail);
    actions::close_panel(ui, state);
    let empty_text = empty_text(&results);
    let mut errors = results.errors;
    errors.extend(state.desktop_error.clone());
    errors.extend(state.hotkey_error.clone());
    let status = if !errors.is_empty() {
        errors.join("；")
    } else if let Some(name) = results.plugin_name {
        i18n::format(
            "status.plugin",
            &[
                ("name", &name),
                ("count", &results.entries.len().to_string()),
            ],
        )
    } else if let Some((ready, total)) = results.partial {
        i18n::format(
            "status.partial",
            &[
                ("ready", &ready.to_string()),
                ("total", &total.to_string()),
                ("count", &results.entries.len().to_string()),
            ],
        )
    } else {
        i18n::format(
            "status.ready",
            &[("count", &results.entries.len().to_string())],
        )
    };
    let rows: Vec<_> = results
        .entries
        .iter()
        .map(|entry| {
            let (icon, native_path, native_image) = icons.resolve(&entry.icon);
            ResultData {
                title: entry.name.clone().into(),
                description: entry.description.clone().into(),
                icon,
                native_path,
                native_image,
                draggable: desktop::drag::SUPPORTED
                    && desktop::drag::path(&entry.resource).is_some(),
                query: results.query.clone().into(),
                title_highlighted: crate::highlight::any(&entry.name, &results.query),
                description_highlighted: crate::highlight::any(&entry.description, &results.query),
            }
        })
        .collect();
    let same_query = state.displayed_query == ui.get_query().as_str();
    let selected = if same_query {
        refreshed_selection(&state.entries, &results.entries, ui.get_selected())
    } else {
        ui.invoke_reset_scroll();
        0
    };
    state.displayed_query = ui.get_query().to_string();
    state.entries = results.entries;
    ui.set_selected(selected);
    actions::sync_selection(ui, state);
    ui.set_results(Rc::new(VecModel::from(rows)).into());
    ui.set_empty_text(empty_text.into());
    ui.set_failed(!errors.is_empty());
    ui.set_status(status.into());
    ui.set_searching(false);
    ui.set_search_running(false);
}

/// What the empty list says: who answered nothing, and why when that is known.
fn empty_text(results: &SearchResults) -> String {
    let query = results.query.as_str();
    let failed = !results.errors.is_empty();
    match (&results.plugin_name, failed, query.is_empty()) {
        (Some(name), true, _) => i18n::format("search.empty_plugin_failed", &[("name", name)]),
        (Some(name), false, true) => i18n::format("search.empty_plugin_blank", &[("name", name)]),
        (Some(name), false, false) => {
            i18n::format("search.empty_plugin", &[("name", name), ("query", query)])
        }
        (None, true, _) => i18n::t("search.empty_failed"),
        (None, false, true) => i18n::t("search.empty_blank"),
        (None, false, false) => match results.partial {
            Some((ready, total)) => i18n::format(
                "search.empty_partial",
                &[("query", query), ("count", &(total - ready).to_string())],
            ),
            None => i18n::format("search.empty", &[("query", query)]),
        },
    }
}

/// Refreshes of the same query keep the selected resource; new queries select the first result.
fn refreshed_selection(previous: &[Entry], next: &[Entry], selected: i32) -> i32 {
    let index = usize::try_from(selected).unwrap_or_default();
    previous
        .get(index)
        .and_then(|entry| {
            next.iter().position(|candidate| {
                candidate.resource == entry.resource && candidate.name == entry.name
            })
        })
        .unwrap_or_else(|| index.min(next.len().saturating_sub(1))) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use mytools_plugin_api::Resource;

    fn empty(plugin: Option<&str>, query: &str) -> SearchResults {
        SearchResults {
            entries: vec![],
            errors: vec![],
            plugin_name: plugin.map(str::to_owned),
            view: Default::default(),
            revision: None,
            cancelled: false,
            waiting: false,
            partial: None,
            query: query.into(),
        }
    }

    #[test]
    fn empty_list_says_who_returned_nothing() {
        let scoped = empty(Some("崩溃测试"), "1");
        assert_eq!(
            empty_text(&scoped),
            i18n::format(
                "search.empty_plugin",
                &[("name", "崩溃测试"), ("query", "1")]
            )
        );
        assert_eq!(
            empty_text(&empty(Some("崩溃测试"), "")),
            i18n::format("search.empty_plugin_blank", &[("name", "崩溃测试")])
        );
        let failed = SearchResults {
            errors: vec!["boom".into()],
            ..scoped
        };
        assert_eq!(
            empty_text(&failed),
            i18n::format("search.empty_plugin_failed", &[("name", "崩溃测试")])
        );
        assert_eq!(
            empty_text(&empty(None, "abc")),
            i18n::format("search.empty", &[("query", "abc")])
        );
        let loading = SearchResults {
            partial: Some((5, 7)),
            ..empty(None, "abc")
        };
        assert_eq!(
            empty_text(&loading),
            i18n::format("search.empty_partial", &[("query", "abc"), ("count", "2")])
        );
        assert_eq!(empty_text(&empty(None, "")), i18n::t("search.empty_blank"));
    }

    fn entries(names: &[&str]) -> Vec<Entry> {
        names
            .iter()
            .map(|name| {
                Entry::new(
                    (*name).into(),
                    String::new(),
                    "builtin:code".into(),
                    Resource::Text {
                        text: (*name).into(),
                    },
                    None,
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn repeated_results_keep_fourth_decode_result_selected() {
        let previous = entries(&["base64", "base64url", "url", "hex", "html", "json"]);
        let next = entries(&["base64", "base64url", "url", "hex", "html", "json"]);
        assert_eq!(refreshed_selection(&previous, &next, 3), 3);
    }

    #[test]
    fn refresh_follows_reordered_resources_and_handles_removed_results() {
        let previous = entries(&["a", "b", "c"]);
        assert_eq!(
            refreshed_selection(&previous, &entries(&["c", "a", "b"]), 2),
            0
        );
        assert_eq!(refreshed_selection(&previous, &entries(&["a", "b"]), 2), 1);
        assert_eq!(refreshed_selection(&previous, &[], 2), 0);
        assert_eq!(refreshed_selection(&[], &entries(&["a", "b"]), 0), 0);
    }
}
