use crate::{
    I18n, ScriptManager, ScriptRow, i18n, runner,
    store::{Kind, Mode, Script, Store},
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

struct State {
    store: Store,
    scripts: Vec<Script>,
    original: Option<Script>,
}

pub fn run(id: Option<String>) -> anyhow::Result<()> {
    let store = Store::new()?;
    let scripts = store.load()?;
    let ui = ScriptManager::new()?;
    crate::fonts::prefer_ui_font_for_chinese();
    ui.global::<I18n>().on_text(|key| i18n::t(&key).into());
    ui.set_windows(cfg!(windows));
    let state = Rc::new(RefCell::new(State {
        store,
        scripts,
        original: None,
    }));
    refresh(&ui, &state.borrow());
    // Open the requested script, else the first one; an empty store starts a new script.
    let initial = {
        let scripts = &state.borrow().scripts;
        id.and_then(|id| scripts.iter().find(|s| s.id == id))
            .or_else(|| scripts.first())
            .cloned()
    };
    load(&ui, &mut state.borrow_mut(), initial);
    ui.set_status(
        i18n::t(if cfg!(windows) {
            "manage.hint"
        } else {
            "platform.unsupported"
        })
        .into(),
    );
    let weak = ui.as_weak();
    let state_copy = state.clone();
    ui.on_choose(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        if !can_leave(&ui) {
            return;
        }
        let mut state = state_copy.borrow_mut();
        let script = usize::try_from(index)
            .ok()
            .and_then(|i| state.scripts.get(i))
            .cloned();
        load(&ui, &mut state, script);
    });
    let weak = ui.as_weak();
    let state_copy = state.clone();
    ui.on_create(move || {
        let Some(ui) = weak.upgrade() else { return };
        if can_leave(&ui) {
            load(&ui, &mut state_copy.borrow_mut(), None);
        }
    });
    let weak = ui.as_weak();
    let state_copy = state.clone();
    ui.on_discard(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state_copy.borrow_mut();
        let original = state.original.clone();
        load(&ui, &mut state, original);
        ui.set_status(i18n::t("discarded").into());
    });
    let weak = ui.as_weak();
    let state_copy = state.clone();
    ui.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = state_copy.borrow_mut();
        let script = Script {
            id: state
                .original
                .as_ref()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: ui.get_script_name().trim().into(),
            alias: ui.get_alias().trim().into(),
            kind: if ui.get_kind() == 0 {
                Kind::Ps1
            } else {
                Kind::Cmd
            },
            content: ui.get_content().to_string(),
            directory: ui.get_directory().trim().into(),
            mode: mode_from(ui.get_mode()),
        };
        let result = state
            .store
            .update(script.clone(), state.original.as_ref())
            .and_then(|_| state.store.load());
        match result {
            Ok(scripts) => {
                state.scripts = scripts;
                refresh(&ui, &state);
                load(&ui, &mut state, Some(script));
                ui.set_status(i18n::t("saved").into());
            }
            Err(error) => ui.set_status(i18n::error(error).into()),
        }
    });
    let weak = ui.as_weak();
    let state_copy = state.clone();
    ui.on_remove(move || {
        let Some(ui) = weak.upgrade() else { return };
        if !can_leave(&ui) {
            return;
        }
        if !ui.get_deleting() {
            ui.set_deleting(true);
            ui.set_status(i18n::t("delete.hint").into());
            return;
        }
        let mut state = state_copy.borrow_mut();
        let Some(original) = state.original.clone() else {
            return;
        };
        match state
            .store
            .delete(&original)
            .and_then(|_| state.store.load())
        {
            Ok(scripts) => {
                state.scripts = scripts;
                refresh(&ui, &state);
                load(&ui, &mut state, None);
                ui.set_status(i18n::t("deleted").into());
            }
            Err(error) => ui.set_status(i18n::error(error).into()),
        }
    });
    let weak = ui.as_weak();
    let state_copy = state.clone();
    ui.on_execute(move |admin| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_dirty() || ui.get_busy() {
            return;
        }
        let state = state_copy.borrow();
        let Some(script) = state.original.clone() else {
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
    let weak = ui.as_weak();
    ui.on_browse(move || {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(folder) = crate::dialog::pick_folder(&ui.get_directory()) {
            ui.set_directory(folder.to_string_lossy().as_ref().into());
        }
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade()
            && (!can_leave(&ui) || ui.get_busy())
        {
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        slint::CloseRequestResponse::HideWindow
    });
    ui.run()?;
    Ok(())
}

fn can_leave(ui: &ScriptManager) -> bool {
    if ui.get_dirty() {
        ui.set_status(i18n::t("unsaved").into());
        false
    } else {
        true
    }
}
fn refresh(ui: &ScriptManager, state: &State) {
    ui.set_scripts(ModelRc::new(VecModel::from(
        state
            .scripts
            .iter()
            .map(|s| ScriptRow {
                name: s.name.clone().into(),
                alias: s.alias.clone().into(),
                kind: s.kind.extension().into(),
            })
            .collect::<Vec<_>>(),
    )));
}
fn load(ui: &ScriptManager, state: &mut State, script: Option<Script>) {
    ui.set_selected(
        script
            .as_ref()
            .and_then(|s| state.scripts.iter().position(|v| v.id == s.id))
            .map(|i| i as i32)
            .unwrap_or(-1),
    );
    let field = |value: fn(&Script) -> &str| -> slint::SharedString {
        script.as_ref().map(value).unwrap_or("").into()
    };
    let (name, alias, content, directory) = (
        field(|s| &s.name),
        field(|s| &s.alias),
        field(|s| &s.content),
        field(|s| &s.directory),
    );
    let kind = i32::from(script.as_ref().is_some_and(|s| s.kind == Kind::Cmd));
    let mode = mode_index(script.as_ref().map(|s| s.mode).unwrap_or_default());
    // The saved copies define "unchanged"; the form is dirty once it differs from them.
    ui.set_saved_name(name.clone());
    ui.set_saved_alias(alias.clone());
    ui.set_saved_kind(kind);
    ui.set_saved_content(content.clone());
    ui.set_saved_directory(directory.clone());
    ui.set_saved_mode(mode);
    ui.set_script_name(name);
    ui.set_alias(alias);
    ui.set_kind(kind);
    ui.set_content(content);
    ui.set_directory(directory);
    ui.set_mode(mode);
    ui.set_saved(script.is_some());
    ui.set_deleting(false);
    state.original = script;
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
