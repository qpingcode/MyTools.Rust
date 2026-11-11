//! Plugin commands run by their global shortcut: the launcher window stays as it is, the
//! result goes to the application that was in front, and problems appear as notices.
use crate::{desktop, i18n, notices::Notifier};
use anyhow::Result;
use mytools_core::{Action, ActionEffect, Notice};
use std::rc::Rc;

/// Runs a command for its shortcut. Call it on the UI thread as soon as the shortcut fires,
/// so the application in front is still the one to paste into.
pub fn run(action: Action, notifier: &Rc<Notifier>) {
    desktop::remember_target();
    let notifier = notifier.clone();
    let spawned = slint::spawn_local(async move {
        if let Err(error) = execute(&action).await {
            let command = action.title.clone().unwrap_or_default();
            mytools_plugin_api::log::warn(
                "command.failed",
                format!("shortcut command {command} failed: {error:#}"),
            );
            notifier.show(vec![Notice {
                title: i18n::format("notice.command", &[("command", &command)]),
                message: format!("{error:#}"),
            }]);
        }
    });
    if let Err(error) = spawned {
        mytools_plugin_api::log::error("command.failed", format!("{error}"));
    }
}

async fn execute(action: &Action) -> Result<()> {
    let (sender, finished) = async_channel::bounded(1);
    let worker = action.clone();
    // The plugin may still be starting, so wait off the UI thread.
    std::thread::spawn(move || {
        let _ = sender.send_blocking(worker.execute());
    });
    let effect = finished
        .recv()
        .await
        .map_err(|_| anyhow::anyhow!(i18n::t("status.command_lost")))??;
    match effect {
        ActionEffect::Plugin(effect) => {
            if let Some(text) = effect.copy_text {
                arboard::Clipboard::new()?.set_text(text)?;
            }
            if effect.paste {
                desktop::paste_into_target().await?;
            }
        }
        ActionEffect::CopyText(text) => arboard::Clipboard::new()?.set_text(text)?,
        ActionEffect::Completed | ActionEffect::Retry(_) => {}
    }
    Ok(())
}
