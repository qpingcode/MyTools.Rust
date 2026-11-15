use super::{close_panel, start_action};
use crate::{
    ActionData, MainWindow,
    action_shortcuts::{Shortcuts, Target},
    app::State,
    i18n,
};
use mytools_core::Action;
use mytools_plugin_api::matching;
use slint::{ComponentHandle, VecModel};
use std::{cell::RefCell, rc::Rc};
pub(crate) struct Panel {
    actions: Vec<Action>,
    rows: Vec<ActionData>,
    visible: Vec<usize>,
    query: String,
    first: usize,
}

impl Panel {
    fn filter(&mut self, query: &str) -> Vec<ActionData> {
        self.query = query.to_owned();
        let query = matching::normalize(query);
        let mut matches: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .skip(self.first)
            .filter_map(|(index, row)| {
                if query.is_empty() {
                    Some((0, index))
                } else {
                    matching::find(&row.title, &query).map(|found| (found.tier, index))
                }
            })
            .collect();
        matches.sort_unstable();
        self.visible = matches.into_iter().map(|(_, index)| index).collect();
        self.visible
            .iter()
            .map(|&index| {
                let mut row = self.rows[index].clone();
                row.query = query.as_str().into();
                row.highlighted = crate::highlight::any(&row.title, &query);
                row
            })
            .collect()
    }

    fn show(&mut self, ui: &MainWindow, query: &str) {
        let rows = self.filter(query);
        ui.set_selected_action(0);
        ui.set_actions(Rc::new(VecModel::from(rows)).into());
    }

    pub(super) fn action(&self, index: i32) -> Option<&Action> {
        self.visible
            .get(usize::try_from(index).ok()?)
            .and_then(|&index| self.actions.get(index))
    }

    fn refresh_shortcuts(&mut self, shortcuts: &Shortcuts) {
        for (row, action) in self.rows.iter_mut().zip(&self.actions) {
            let target = Target::for_action(action, &self.actions);
            row.bindable = target.is_some();
            row.shortcut = target
                .as_ref()
                .and_then(|target| shortcuts.get(target))
                .unwrap_or_default()
                .into();
        }
    }
}

pub(super) fn wire_panel(ui: &MainWindow, state: &Rc<RefCell<State>>) {
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
        // In chat Enter sends a message, so it does not replace any result action.
        let first = usize::from(!ui.get_chat_view());
        if entry.actions.len() <= first {
            return;
        }
        let snapshot = entry.actions.clone();
        let rows: Vec<_> = snapshot
            .iter()
            .map(|action| ActionData {
                title: i18n::action_title(action).into(),
                icon: action.icon.clone().into(),
                ..Default::default()
            })
            .collect();
        ui.set_action_target(entry.name.clone().into());
        let mut panel = Panel {
            actions: snapshot,
            rows,
            visible: Vec::new(),
            query: String::new(),
            first,
        };
        panel.refresh_shortcuts(&state.action_shortcuts);
        panel.show(&ui, "");
        state.panel = Some(panel);
        ui.set_actions_open(true);
    });
    let weak = ui.as_weak();
    let filter_state = state.clone();
    ui.on_filter_actions(move |query| {
        if let Some(ui) = weak.upgrade()
            && let Some(panel) = filter_state.borrow_mut().panel.as_mut()
        {
            panel.show(&ui, &query);
        }
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
            if let Some(panel) = &state.panel {
                let last = panel.visible.len().saturating_sub(1) as i32;
                ui.set_selected_action((ui.get_selected_action() + delta).clamp(0, last));
            }
        }
    });
}

pub(super) fn wire_shortcuts(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    let weak = ui.as_weak();
    let edit_state = state.clone();
    ui.on_assign_action_shortcut(move |index, text| {
        let Some(ui) = weak.upgrade() else {
            return false;
        };
        let mut state = edit_state.borrow_mut();
        let State {
            panel,
            action_shortcuts,
            global_command_shortcuts,
            ..
        } = &mut *state;
        let Some(panel) = panel.as_mut() else {
            return false;
        };
        let Some(action) = panel.action(index) else {
            return false;
        };
        let Some(target) = Target::for_action(action, &panel.actions) else {
            return false;
        };
        if let Err(message) = action_shortcuts.assign(
            target,
            &i18n::action_title(action),
            &text,
            global_command_shortcuts,
        ) {
            ui.set_action_shortcut_error(message.into());
            return false;
        }
        panel.refresh_shortcuts(action_shortcuts);
        let query = panel.query.clone();
        panel.show(&ui, &query);
        ui.set_selected_action(index);
        true
    });
    let weak = ui.as_weak();
    let state = state.clone();
    ui.on_run_action_shortcut(move |text, ctrl, shift, alt, meta| {
        let Some(ui) = weak.upgrade() else {
            return false;
        };
        if ui.get_action_busy()
            || ui.get_searching()
            || (!ctrl && !shift && !alt && !meta && text.is_ascii())
        {
            return false;
        }
        let mut state = state.borrow_mut();
        let Some(entry) = usize::try_from(ui.get_selected())
            .ok()
            .and_then(|index| state.entries.get(index))
        else {
            return false;
        };
        let text = text.to_lowercase();
        let Some(action) = entry
            .actions
            .iter()
            .find(|action| {
                Target::for_action(action, &entry.actions).is_some_and(|target| {
                    state
                        .action_shortcuts
                        .matches(&target, &text, (ctrl, shift, alt, meta))
                })
            })
            .cloned()
        else {
            return false;
        };
        close_panel(&ui, &mut state);
        crate::attachments::consume(&ui, &mut state);
        start_action(&ui, action);
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mytools_plugin_api::{ActionId, Resource};

    #[test]
    fn filtered_rows_execute_the_matching_action() {
        let actions = mytools_core::resolve_actions(
            &Resource::Url {
                url: "https://example.com".into(),
            },
            None,
        )
        .unwrap();
        let mut panel = Panel {
            actions,
            rows: vec![
                ActionData {
                    title: "Open in browser".into(),
                    ..Default::default()
                },
                ActionData {
                    title: "复制路径".into(),
                    ..Default::default()
                },
            ],
            visible: Vec::new(),
            query: String::new(),
            first: 1,
        };
        for query in ["复制", "fzlj", "fuzhilujing"] {
            let results = panel.filter(query);
            assert_eq!(results.len(), 1);
            assert!(results[0].highlighted);
            assert_eq!(panel.action(0).unwrap().id, ActionId::CopyText);
        }
        assert!(panel.action(1).is_none());
        assert!(panel.filter("missing").is_empty());
        assert!(panel.action(0).is_none());
        assert!(panel.filter("browser").is_empty());
        assert_eq!(panel.filter("").len(), 1);
        assert_eq!(panel.action(0).unwrap().id, ActionId::CopyText);

        // Chat displays every action because the Enter button sends messages.
        panel.first = 0;
        assert_eq!(panel.filter("").len(), 2);
        assert_eq!(panel.action(0).unwrap().id, ActionId::OpenInBrowser);
    }
}
