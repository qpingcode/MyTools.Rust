use crate::{
    I18n, ScriptManager, ScriptRow,
    editor::{self, Editor},
    i18n, runner,
    store::{Kind, Mode, Script, Store},
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

struct State {
    store: Store,
    editor: Editor,
    /// The ids of the sidebar rows, in their order.
    order: Vec<String>,
}

pub fn run(id: Option<String>) -> anyhow::Result<()> {
    let store = Store::new()?;
    let editor = Editor::new(store.load()?, store.sort_by_name());
    let ui = ScriptManager::new()?;
    crate::fonts::prefer_ui_font_for_chinese();
    ui.global::<I18n>().on_text(|key| i18n::t(&key).into());
    ui.set_windows(cfg!(windows));
    ui.set_sort_by_name(editor.sort_by_name);
    let state = Rc::new(RefCell::new(State {
        store,
        editor,
        order: Vec::new(),
    }));
    // Open the requested script, else the first one; an empty store starts a new script.
    let initial = {
        let editor = &state.borrow().editor;
        id.filter(|id| editor.original(id).is_some())
            .or_else(|| editor.rows().first().map(|s| s.id.clone()))
    };
    show(&ui, &mut state.borrow_mut(), initial);
    ui.set_status(
        i18n::t(if cfg!(windows) {
            "manage.hint"
        } else {
            "platform.unsupported"
        })
        .into(),
    );
    connect_editing(&ui, &state);
    connect_actions(&ui, &state);
    connect_closing(&ui, &state);
    ui.run()?;
    Ok(())
}

/// Choosing, creating, discarding and sorting scripts; none of these saves anything.
fn connect_editing(ui: &ScriptManager, state: &Rc<RefCell<State>>) {
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_choose(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state_copy.borrow_mut();
        stash(&ui, &mut state);
        let id = usize::try_from(index)
            .ok()
            .and_then(|i| state.order.get(i))
            .cloned();
        show(&ui, &mut state, id);
    });
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_create(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state_copy.borrow_mut();
        stash(&ui, &mut state);
        show(&ui, &mut state, None);
    });
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_discard(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state_copy.borrow_mut();
        let Some(id) = state.editor.current.clone() else {
            return;
        };
        // A new script goes back to blank rather than disappearing from under the form.
        let baseline = state.editor.baseline(&id);
        state.editor.discard(&id);
        if state.editor.original(&id).is_none() {
            state.editor.put(baseline);
        }
        show(&ui, &mut state, Some(id));
        ui.set_status(i18n::t("discarded").into());
    });
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_toggle_sort(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state_copy.borrow_mut();
        stash(&ui, &mut state);
        let sort_by_name = !state.editor.sort_by_name;
        state.editor.sort_by_name = sort_by_name;
        ui.set_sort_by_name(sort_by_name);
        refresh(&ui, &mut state);
        ui.set_status(match state.store.set_sort_by_name(sort_by_name) {
            Ok(()) => i18n::t(if sort_by_name {
                "sort.name"
            } else {
                "sort.created"
            })
            .into(),
            Err(error) => i18n::error(error).into(),
        });
    });
    let weak = ui.as_weak();
    ui.on_browse(move || {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(folder) = crate::dialog::pick_folder(&ui.get_directory()) {
            ui.set_directory(folder.to_string_lossy().as_ref().into());
        }
    });
}

/// Saving, deleting and running, which act on the store or the system.
fn connect_actions(ui: &ScriptManager, state: &Rc<RefCell<State>>) {
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        save_all(&ui, &mut state_copy.borrow_mut());
    });
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_remove(move || {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.get_deleting() {
            ui.set_deleting(true);
            ui.set_status(i18n::t("delete.hint").into());
            return;
        }
        let mut state = state_copy.borrow_mut();
        let Some(original) = state
            .editor
            .current
            .as_deref()
            .and_then(|id| state.editor.original(id))
            .cloned()
        else {
            return;
        };
        match state
            .store
            .delete(&original)
            .and_then(|_| state.store.load())
        {
            Ok(scripts) => {
                state.editor.saved = scripts;
                state.editor.discard(&original.id);
                state.editor.current = None;
                let next = state.editor.rows().first().map(|s| s.id.clone());
                show(&ui, &mut state, next);
                ui.set_status(i18n::t("deleted").into());
            }
            Err(error) => ui.set_status(i18n::error(error).into()),
        }
    });
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_execute(move |admin| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_dirty() || ui.get_busy() {
            return;
        }
        let state = state_copy.borrow();
        let Some(script) = state
            .editor
            .current
            .as_deref()
            .and_then(|id| state.editor.original(id))
            .cloned()
        else {
            return;
        };
        let root = state.store.root.clone();
        ui.set_busy(true);
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = runner::run(&Store { root }, &script, admin);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_busy(false);
                ui.set_status(
                    match result {
                        Ok(()) => i18n::t("run.started"),
                        Err(error) => i18n::error(error),
                    }
                    .into(),
                );
            });
        });
    });
}

/// Closing with unsaved scripts asks whether to save them first.
fn connect_closing(ui: &ScriptManager, state: &Rc<RefCell<State>>) {
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.window().on_close_requested(move || {
        let Some(ui) = weak.upgrade() else {
            return slint::CloseRequestResponse::HideWindow;
        };
        if ui.get_busy() {
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        let mut state = state_copy.borrow_mut();
        stash(&ui, &mut state);
        let count = state.editor.dirty_count(None);
        if count == 0 {
            return slint::CloseRequestResponse::HideWindow;
        }
        ui.set_close_message(
            i18n::format("close.message", &[("count", &count.to_string())]).into(),
        );
        ui.set_close_prompt(true);
        slint::CloseRequestResponse::KeepWindowShown
    });
    let (weak, state_copy) = (ui.as_weak(), state.clone());
    ui.on_close_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_close_prompt(false);
        if save_all(&ui, &mut state_copy.borrow_mut()) {
            quit(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_close_discard(move || {
        if let Some(ui) = weak.upgrade() {
            quit(&ui);
        }
    });
    let weak = ui.as_weak();
    ui.on_close_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_close_prompt(false);
        }
    });
}

fn quit(ui: &ScriptManager) {
    let _ = ui.hide();
    let _ = slint::quit_event_loop();
}

/// Saves every changed script together. When one is invalid nothing is saved and that
/// script is opened with the reason. Returns whether everything was saved.
fn save_all(ui: &ScriptManager, state: &mut State) -> bool {
    stash(ui, state);
    let changes = state.editor.changes();
    if changes.is_empty() {
        return true;
    }
    if let Some((script, error)) = changes
        .iter()
        .find_map(|(script, _)| script.validate().err().map(|error| (script, error)))
    {
        let name = match script.name.as_str() {
            "" => i18n::t("new"),
            name => name.to_owned(),
        };
        let id = script.id.clone();
        show(ui, state, Some(id));
        ui.set_status(
            i18n::format(
                "save.invalid",
                &[("name", &name), ("error", &error.to_string())],
            )
            .into(),
        );
        return false;
    }
    match state
        .store
        .save_all(&changes)
        .and_then(|_| state.store.load())
    {
        Ok(scripts) => {
            state.editor.saved_all(scripts);
            let current = state.editor.current.clone();
            show(ui, state, current);
            ui.set_status(match changes.len() {
                1 => i18n::t("saved").into(),
                count => i18n::format("saved.count", &[("count", &count.to_string())]).into(),
            });
            true
        }
        Err(error) => {
            ui.set_status(i18n::error(error).into());
            false
        }
    }
}

/// Keeps the form's contents as the current script's draft.
fn stash(ui: &ScriptManager, state: &mut State) {
    let Some(id) = state.editor.current.clone() else {
        return;
    };
    let created = state.editor.get(&id).map_or(0, |s| s.created);
    state.editor.put(Script {
        id,
        name: ui.get_script_name().into(),
        alias: ui.get_alias().into(),
        kind: kind_from(ui.get_kind()),
        content: ui.get_content().into(),
        directory: ui.get_directory().into(),
        mode: mode_from(ui.get_mode()),
        created,
    });
}

/// Opens a script in the form, or a new one for `None`.
fn show(ui: &ScriptManager, state: &mut State, id: Option<String>) {
    match id.filter(|id| state.editor.get(id).is_some()) {
        Some(id) => state.editor.select(Some(id)),
        None => {
            state.editor.select(None);
            state.editor.create(editor::now());
        }
    }
    let Some(id) = state.editor.current.clone() else {
        return;
    };
    let Some(script) = state.editor.get(&id).cloned() else {
        return;
    };
    let baseline = state.editor.baseline(&id);
    // The baseline defines "unchanged"; the form is dirty once it differs from it.
    ui.set_saved_name(baseline.name.into());
    ui.set_saved_alias(baseline.alias.into());
    ui.set_saved_kind(kind_index(baseline.kind));
    ui.set_saved_content(baseline.content.into());
    ui.set_saved_directory(baseline.directory.into());
    ui.set_saved_mode(mode_index(baseline.mode));
    ui.set_script_name(script.name.into());
    ui.set_alias(script.alias.into());
    ui.set_kind(kind_index(script.kind));
    ui.set_content(script.content.into());
    ui.set_directory(script.directory.into());
    ui.set_mode(mode_index(script.mode));
    ui.set_saved(state.editor.original(&id).is_some());
    if ui.get_deleting() {
        ui.set_deleting(false);
        ui.set_status("".into());
    }
    refresh(ui, state);
}

/// Lists the scripts in the sidebar and marks the current one.
fn refresh(ui: &ScriptManager, state: &mut State) {
    let editor = &state.editor;
    let rows = editor.rows();
    let current = editor.current.as_deref();
    ui.set_selected(
        rows.iter()
            .position(|s| Some(s.id.as_str()) == current)
            .map_or(-1, |i| i as i32),
    );
    ui.set_other_unsaved(editor.dirty_count(current) as i32);
    ui.set_scripts(ModelRc::new(VecModel::from(
        rows.iter()
            .map(|s| ScriptRow {
                name: s.name.clone().into(),
                alias: s.alias.clone().into(),
                kind: s.kind.name().into(),
                dirty: editor.is_dirty(&s.id),
            })
            .collect::<Vec<_>>(),
    )));
    state.order = rows.iter().map(|s| s.id.clone()).collect();
}

/// The order of the type options in the form.
const KINDS: [Kind; 3] = [Kind::Pwsh, Kind::Ps1, Kind::Cmd];
fn kind_index(kind: Kind) -> i32 {
    KINDS.iter().position(|k| *k == kind).unwrap_or(0) as i32
}
fn kind_from(index: i32) -> Kind {
    usize::try_from(index)
        .ok()
        .and_then(|i| KINDS.get(i).copied())
        .unwrap_or_default()
}
fn mode_index(mode: Mode) -> i32 {
    match mode {
        Mode::Window => 0,
        Mode::CloseOnSuccess => 1,
        Mode::Background => 2,
    }
}
fn mode_from(index: i32) -> Mode {
    match index {
        1 => Mode::CloseOnSuccess,
        2 => Mode::Background,
        _ => Mode::Window,
    }
}
