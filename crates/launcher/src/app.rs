use crate::{
    AboutWindow, AppTray, I18n, MainWindow, ResultData, TextFit, Theme, actions, config, desktop,
    i18n, icons,
    search::{Query, SearchWorker},
    theme, tray,
};
use mytools_core::{Action, Entry, SearchResults};
use slint::{ComponentHandle, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    displayed_query: String,
    pub entries: Vec<Entry>,
    pub panel: Option<Vec<Action>>,
    desktop_error: Option<String>,
}

pub fn run() -> anyhow::Result<()> {
    let config = config::load();
    let plugin_shortcuts = config
        .as_ref()
        .map(|config| config.plugin_shortcuts.clone())
        .unwrap_or_default();
    let requested = config
        .as_ref()
        .map(|config| config.language.as_str())
        .unwrap_or("auto");
    mytools_plugin_api::i18n::initialize(requested);
    let theme_errors = theme::configure(
        config
            .as_ref()
            .map(|config| config.theme.clone())
            .unwrap_or_default(),
    );
    desktop::configure_backend()?;
    let ui = MainWindow::new()?;
    theme::apply_configured(&ui.global::<Theme>());
    let position_tracker = crate::window_position::Tracker::new(&ui);
    let tray = AppTray::new()?;
    let about = AboutWindow::new()?;
    theme::apply_configured(&about.global::<Theme>());
    ui.global::<I18n>().on_text(|key| i18n::t(&key).into());
    ui.global::<TextFit>()
        .on_path(|text, width, font_size| crate::elide::fit_path(&text, width, font_size).into());
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
    let mut initial = State::default();
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
    let state = Rc::new(RefCell::new(initial));
    let icons = icons::Loader::new(&ui)?;
    let (worker, responses) = SearchWorker::start();
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
    let _responses = slint::spawn_local(async move {
        while let Ok(completed) = responses.recv().await {
            let Some(ui) = weak.upgrade() else { break };
            let mut state = response_state.borrow_mut();
            if completed.generation == state.generation {
                apply_results(&ui, &mut state, completed.results, &icons);
            }
        }
    })?;

    let desktop = match desktop::Desktop::new(&ui, &plugin_shortcuts) {
        Ok(desktop) => {
            if !desktop.registration_errors.is_empty() {
                state.borrow_mut().desktop_error = Some(desktop.registration_errors.join("；"));
                desktop::show(&ui);
            }
            Some(desktop)
        }
        Err(error) => {
            state.borrow_mut().desktop_error = Some(format!("{error:#}"));
            desktop::show(&ui);
            None
        }
    };
    ui.invoke_search("".into());
    if std::env::args().any(|argument| argument == "--show") {
        desktop::show(&ui);
    }
    tray.show()?;
    // Hiding the launcher must never terminate the resident application.
    let result = slint::run_event_loop_until_quit();
    drop(desktop);
    drop(worker);
    result?;
    let restart = tray_controller.restart_requested();
    drop(tray_controller);
    drop(tray);
    drop(about);
    drop(position_tracker);
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
    let mut errors = results.errors;
    errors.extend(state.desktop_error.clone());
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
    ui.set_failed(!errors.is_empty());
    ui.set_status(status.into());
    ui.set_searching(false);
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
