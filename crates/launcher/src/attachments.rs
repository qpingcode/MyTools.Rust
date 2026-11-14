use crate::{MainWindow, app::State, desktop, i18n, notices::Notifier};
use mytools_core::Notice;
use mytools_plugin_api::CommandArgument;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub fn wire(
    ui: &MainWindow,
    state: &Rc<RefCell<State>>,
    notifier: Rc<Notifier>,
) -> anyhow::Result<impl Fn(desktop::selection::Source) + Send + Sync + 'static> {
    let weak = ui.as_weak();
    let normal_state = state.clone();
    ui.on_normal_launch(move || {
        let Some(ui) = weak.upgrade() else { return };
        let removed = {
            let mut state = normal_state.borrow_mut();
            state.attachment_open = state.attachment_open.wrapping_add(1);
            state.attachment.take().is_some()
        };
        if removed {
            ui.set_attachment_label("".into());
            ui.set_attachment_details("".into());
            ui.invoke_search(ui.get_query());
        }
    });
    let weak = ui.as_weak();
    ui.on_detach_attachment(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_normal_launch();
            ui.invoke_focus_search();
        }
    });
    let weak = ui.as_weak();
    let state = state.clone();
    // Coalesce repeats while a capture is running, especially during Ctrl+C fallback.
    let (sender, receiver) = async_channel::bounded::<desktop::selection::Source>(1);
    slint::spawn_local(async move {
        while let Ok(source) = receiver.recv().await {
            let Some(ui) = weak.upgrade() else { break };
            let ticket = state.borrow().attachment_open;
            let (done, result) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = done.send_blocking(source.capture());
            });
            let Ok(result) = result.recv().await else {
                continue;
            };
            while receiver.try_recv().is_ok() {}
            if ticket != state.borrow().attachment_open {
                continue;
            }
            let attachment = match result {
                Ok(value) => value,
                Err(error) => {
                    mytools_plugin_api::log::warn("selection.capture_failed", format!("{error:#}"));
                    None
                }
            };
            if attachment.is_none() {
                notifier.show(vec![Notice {
                    title: i18n::t("attachment.title"),
                    message: i18n::t("attachment.empty"),
                }]);
            }
            let (label, details) = attachment
                .as_ref()
                .map(attachment_display)
                .unwrap_or_default();
            state.borrow_mut().attachment = attachment.map(Arc::new);
            ui.set_attachment_label(label.into());
            ui.set_attachment_details(details.into());
            ui.set_detail_view(false);
            ui.set_editor_view(false);
            ui.set_chat_view(false);
            ui.set_query("".into());
            ui.invoke_search("".into());
            desktop::show_window(&ui);
        }
    })?;
    Ok(move |source| {
        let _ = sender.try_send(source);
    })
}

fn attachment_display(attachment: &CommandArgument) -> (String, String) {
    match attachment {
        CommandArgument::Text { text } => {
            let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let preview: String = line.chars().take(100).collect();
            let label = if line.len() > preview.len() {
                format!("{preview}…")
            } else {
                preview
            };
            (
                i18n::format("attachment.text", &[("text", &label)]),
                text.clone(),
            )
        }
        CommandArgument::Files { paths } => (
            i18n::format("attachment.files", &[("count", &paths.len().to_string())]),
            paths
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    }
}
