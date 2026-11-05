use crate::{ActionData, MainWindow, app::State, desktop, i18n};
use mytools_core::{Action, ActionEffect};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

pub fn wire(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    wire_execution(ui, state);
    wire_panel(ui, state);
    let weak = ui.as_weak();
    let selection_state = state.clone();
    ui.on_select_result(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = selection_state.borrow_mut();
        if usize::try_from(index)
            .ok()
            .and_then(|index| state.entries.get(index))
            .is_none()
        {
            return;
        }
        close_panel(&ui, &mut state);
        ui.set_selected(index);
        sync_selection(&ui, &state);
    });
    let weak = ui.as_weak();
    let navigate_state = state.clone();
    ui.on_navigate(move |delta| {
        if let Some(ui) = weak.upgrade() {
            let mut state = navigate_state.borrow_mut();
            close_panel(&ui, &mut state);
            let last = state.entries.len().saturating_sub(1) as i32;
            ui.set_selected((ui.get_selected() + delta).clamp(0, last));
            sync_selection(&ui, &state);
        }
    });
    let weak = ui.as_weak();
    let dismiss_state = state.clone();
    ui.on_dismiss(move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_actions_open() {
                close_panel(&ui, &mut dismiss_state.borrow_mut());
            } else {
                let _ = ui.hide();
            }
        }
    });
    let weak = ui.as_weak();
    ui.on_drag_requested(move || {
        if let Some(ui) = weak.upgrade() {
            desktop::drag(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_resize_requested(move |edge| {
        if let Some(ui) = weak.upgrade() {
            desktop::drag_resize(&ui, &edge);
        }
    });
    let weak = ui.as_weak();
    let close_state = state.clone();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            close_panel(&ui, &mut close_state.borrow_mut());
        }
        slint::CloseRequestResponse::HideWindow
    });
}

fn wire_execution(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    let weak = ui.as_weak();
    let result_state = state.clone();
    ui.on_execute(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_action_busy() || ui.get_searching() {
            return;
        }
        let mut state = result_state.borrow_mut();
        let action = usize::try_from(index)
            .ok()
            .and_then(|i| state.entries.get(i))
            .and_then(|entry| entry.actions.first())
            .cloned();
        if let Some(action) = action {
            ui.set_selected(index);
            close_panel(&ui, &mut state);
            sync_selection(&ui, &state);
            start_action(&ui, action);
        }
    });
    let weak = ui.as_weak();
    let panel_state = state.clone();
    ui.on_execute_action(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_action_busy() || ui.get_searching() {
            return;
        }
        let mut state = panel_state.borrow_mut();
        let action = state
            .panel
            .as_ref()
            .and_then(|actions| usize::try_from(index).ok().and_then(|i| actions.get(i)))
            .cloned();
        if let Some(action) = action {
            close_panel(&ui, &mut state);
            start_action(&ui, action);
        }
    });
}

fn wire_panel(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    let weak = ui.as_weak();
    let toggle_state = state.clone();
    ui.on_toggle_actions(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = toggle_state.borrow_mut();
        if ui.get_actions_open() {
            close_panel(&ui, &mut state);
            return;
        }
        if ui.get_action_busy() || ui.get_searching() {
            return;
        }
        let Some(entry) = usize::try_from(ui.get_selected())
            .ok()
            .and_then(|index| state.entries.get(index))
        else {
            return;
        };
        if entry.actions.len() <= 1 {
            return;
        }
        let snapshot = entry.actions[1..].to_vec();
        let rows: Vec<_> = snapshot
            .iter()
            .map(|action| ActionData {
                title: i18n::action_title(action).into(),
                icon: action.icon.clone().into(),
            })
            .collect();
        ui.set_action_target(entry.name.clone().into());
        state.panel = Some(snapshot);
        ui.set_actions(Rc::new(VecModel::from(rows)).into());
        ui.set_selected_action(0);
        ui.set_actions_open(true);
    });
    let weak = ui.as_weak();
    let close_state = state.clone();
    ui.on_close_actions(move || {
        if let Some(ui) = weak.upgrade() {
            close_panel(&ui, &mut close_state.borrow_mut());
        }
    });
    let weak = ui.as_weak();
    let navigate_state = state.clone();
    ui.on_navigate_action(move |delta| {
        if let Some(ui) = weak.upgrade() {
            let state = navigate_state.borrow();
            if let Some(actions) = &state.panel {
                let last = actions.len().saturating_sub(1) as i32;
                ui.set_selected_action((ui.get_selected_action() + delta).clamp(0, last));
            }
        }
    });
}

pub fn close_panel(ui: &MainWindow, state: &mut State) {
    state.panel = None;
    ui.set_actions_open(false);
    ui.set_actions(ModelRc::default());
    ui.set_selected_action(0);
    ui.invoke_focus_search();
}

pub fn sync_selection(ui: &MainWindow, state: &State) {
    let preview = usize::try_from(ui.get_selected())
        .ok()
        .and_then(|index| state.entries.get(index))
        .and_then(|entry| entry.preview.as_ref());
    ui.set_preview_title(
        preview
            .map(|preview| preview.title.as_str())
            .unwrap_or("")
            .into(),
    );
    let full_text = preview
        .and_then(|preview| preview.text_file.as_ref())
        .filter(|path| path.is_absolute())
        .filter(|path| {
            std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= 2 * 1024 * 1024)
        })
        .and_then(|path| std::fs::read_to_string(path).ok());
    ui.set_preview_text(
        full_text
            .as_deref()
            .or_else(|| preview.map(|preview| preview.text.as_str()))
            .unwrap_or("")
            .into(),
    );
    ui.set_preview_metadata(
        preview
            .map(|preview| preview.metadata.as_str())
            .unwrap_or("")
            .into(),
    );
    let picture = preview
        .and_then(|preview| preview.image.as_ref())
        .filter(|path| path.is_absolute())
        .and_then(|path| slint::Image::load_from_path(path).ok())
        .unwrap_or_default();
    ui.set_preview_image(picture);
    let actions = usize::try_from(ui.get_selected())
        .ok()
        .and_then(|index| state.entries.get(index))
        .map(|entry| entry.actions.as_slice())
        .unwrap_or_default();
    let primary = actions.first();
    ui.set_primary_available(primary.is_some());
    ui.set_actions_available(actions.len() > 1);
    ui.set_primary_title(
        primary
            .map(i18n::action_title)
            .unwrap_or_else(|| i18n::t("actions.execute"))
            .into(),
    );
}

fn start_action(ui: &MainWindow, action: Action) {
    ui.set_action_status(
        i18n::format(
            "actions.running",
            &[("action", &i18n::action_title(&action))],
        )
        .into(),
    );
    ui.set_action_busy(true);
    let query = ui.get_query();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = action.execute();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let message = result.and_then(|effect| match effect {
                ActionEffect::CopyText(text) => {
                    arboard::Clipboard::new()?.set_text(text.clone())?;
                    Ok(i18n::format("status.copied", &[("text", &text)]))
                }
                ActionEffect::Completed => {
                    if action.dismiss_on_success() && ui.get_query() == query {
                        ui.hide()?;
                    }
                    Ok(i18n::format(
                        "status.completed",
                        &[("action", &i18n::action_title(&action))],
                    ))
                }
                ActionEffect::Retry(attempt) => {
                    crate::search::request_retry(attempt);
                    ui.invoke_search(ui.get_query());
                    Ok(i18n::t("status.retrying"))
                }
                ActionEffect::Plugin(effect) => {
                    if let Some(text) = effect.copy_text {
                        arboard::Clipboard::new()?.set_text(text)?;
                    }
                    if effect.paste {
                        desktop::paste(&ui)?;
                    }
                    if effect.refresh {
                        ui.invoke_search(ui.get_query());
                    }
                    if effect.dismiss && ui.get_query() == query {
                        ui.hide()?;
                    }
                    Ok(effect.message)
                }
            });
            if message.is_ok()
                && let Err(error) = action.record_success()
            {
                eprintln!(
                    "{}",
                    i18n::format("history.save_failed", &[("error", &error.to_string())])
                );
            }
            ui.set_action_busy(false);
            // A completed action must not replace the status of a newer search.
            if ui.get_query() == query {
                match message {
                    Ok(message) => {
                        ui.set_failed(false);
                        ui.set_status(message.into());
                    }
                    Err(error) => {
                        ui.set_failed(true);
                        ui.set_status(
                            i18n::format("status.failed", &[("error", &format!("{error:#}"))])
                                .into(),
                        );
                    }
                }
            }
        });
    });
}
