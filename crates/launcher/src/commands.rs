//! Plugin commands run by their global shortcut: the launcher window stays as it is, the
//! result goes to the application that was in front, and problems appear as notices.
//!
//! Commands that work on the selection (such as translating it) first read the text selected
//! in that application. When it is editable the result replaces it; otherwise the plugin's
//! scope opens with the selected text, so the result shows in the launcher.
//!
//! A command that takes a while shows a toast in the lower middle of the screen meanwhile,
//! which says what will happen with the result.
use crate::{MainWindow, desktop, i18n, notices::Notifier, toast::Toast};
use anyhow::Result;
use mytools_core::{Action, ActionEffect, Notice};
use std::{cell::RefCell, rc::Rc, time::Duration};

/// Where a command was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Its global shortcut, with another application in front.
    Shortcut,
    /// A search result; the launcher hid itself first.
    Launcher,
}

struct Context {
    window: slint::Weak<MainWindow>,
    notifier: Rc<Notifier>,
    toast: Rc<Toast>,
}

thread_local! {
    static CONTEXT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Gives commands the window they may open and where to report problems.
pub fn init(ui: &MainWindow, notifier: Rc<Notifier>, font: &str) {
    use slint::ComponentHandle;
    CONTEXT.with_borrow_mut(|context| {
        *context = Some(Context {
            window: ui.as_weak(),
            notifier,
            toast: Toast::new(font),
        })
    });
}

/// Runs a command on the UI thread. For a shortcut, call it as soon as the shortcut fires,
/// so the application in front is still the one to paste into.
pub fn run(action: Action, origin: Origin) {
    if origin == Origin::Shortcut {
        desktop::remember_target();
    }
    let Some((window, notifier, toast)) = CONTEXT.with_borrow(|context| {
        context.as_ref().map(|context| {
            (
                context.window.clone(),
                context.notifier.clone(),
                context.toast.clone(),
            )
        })
    }) else {
        return;
    };
    let spawned = slint::spawn_local(async move {
        let command = action.title.clone().unwrap_or_default();
        let result = if action.needs_selection() {
            run_on_selection(&action, origin, &window, &toast).await
        } else {
            toast.start(running(&command), String::new());
            execute(&action).await
        };
        toast.close();
        if let Err(error) = result {
            mytools_plugin_api::log::warn(
                "command.failed",
                format!("command {command} failed: {error:#}"),
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

fn running(command: &str) -> String {
    i18n::format("toast.running", &[("command", command)])
}

/// Runs `work` on its own thread, e.g. a plugin that may still be starting.
async fn background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (sender, finished) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = sender.send_blocking(work());
    });
    finished
        .recv()
        .await
        .map_err(|_| anyhow::anyhow!(i18n::t("status.command_lost")))?
}

async fn wait(duration: Duration) -> Result<()> {
    background(move || {
        std::thread::sleep(duration);
        Ok(())
    })
    .await
}

async fn execute(action: &Action) -> Result<()> {
    let worker = action.clone();
    let effect = background(move || worker.execute()).await?;
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

async fn run_on_selection(
    action: &Action,
    origin: Origin,
    window: &slint::Weak<MainWindow>,
    toast: &Rc<Toast>,
) -> Result<()> {
    if origin == Origin::Launcher {
        // Give the application its focus back, so its selection is the current one again.
        desktop::activate_target()?;
        wait(Duration::from_millis(150)).await?;
    }
    let optional = action.selection_optional();
    let selection = match background(desktop::selection::capture).await {
        Ok(selection) => selection,
        // A command that also works without a selection runs without one.
        Err(_) if optional => Default::default(),
        Err(error) => return Err(error),
    };
    let text = selection.text.trim().to_owned();
    let result = if text.is_empty() && !optional {
        Err(anyhow::anyhow!(i18n::t("selection.empty")))
    } else {
        let next = if selection.editable {
            "toast.replace"
        } else if action.selection_scope().is_some() {
            "toast.show"
        } else {
            "toast.copy"
        };
        let command = action.title.clone().unwrap_or_default();
        toast.start(running(&command), i18n::t(next));
        let worker = action.with_selection(text.clone(), selection.editable);
        let result = background(move || worker.execute()).await;
        toast.close();
        result
    };
    let effect = match result {
        Ok(ActionEffect::Plugin(effect)) => effect,
        Ok(_) => return restore(selection.previous_clipboard),
        Err(error) => {
            restore(selection.previous_clipboard)?;
            return Err(error);
        }
    };
    if effect.open {
        restore(selection.previous_clipboard)?;
        if let Some(scope) = action.selection_scope() {
            open_scope(window, scope, "");
        }
        return Ok(());
    }
    let Some(output) = effect.copy_text else {
        return restore(selection.previous_clipboard);
    };
    if !effect.paste {
        return Ok(arboard::Clipboard::new()?.set_text(output)?);
    }
    if selection.editable && desktop::target_in_front() {
        // What the user had copied comes back once the result was pasted.
        let previous = match selection.previous_clipboard {
            Some(previous) => previous,
            None => arboard::Clipboard::new()?.get_text().ok(),
        };
        set_private(desktop::selection::replacement(&selection.text, &output))?;
        let pasted = desktop::paste_into_target().await;
        wait(Duration::from_millis(500)).await?;
        restore(Some(previous))?;
        return pasted;
    }
    restore(selection.previous_clipboard)?;
    match action.selection_scope() {
        Some(scope) => open_scope(window, scope, &text),
        None => arboard::Clipboard::new()?.set_text(output)?,
    }
    Ok(())
}

/// Shows the plugin's scope with `text` in its input, as if typed there.
fn open_scope(window: &slint::Weak<MainWindow>, scope: &str, text: &str) {
    let Some(ui) = window.upgrade() else {
        return;
    };
    desktop::show(&ui);
    // A conversation that already shows does not appear again to take the focus, which
    // `show` gave to the search box, still before the alias.
    let conversation_shown = ui.get_chat_view() && ui.get_query() == scope;
    ui.set_query(scope.into());
    ui.invoke_search(scope.into());
    ui.set_editor_text(text.into());
    ui.invoke_editor_edited(text.into());
    if conversation_shown {
        ui.invoke_focus_chat();
    } else {
        ui.invoke_focus_search_at_end();
    }
}

/// Sets text that clipboard histories (Windows' and the clipboard plugin's) should skip.
fn set_private(text: String) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    #[cfg(windows)]
    {
        use arboard::SetExtWindows;
        clipboard
            .set()
            .exclude_from_monitoring()
            .exclude_from_history()
            .text(text)?;
    }
    #[cfg(not(windows))]
    clipboard.set_text(text)?;
    Ok(())
}

/// Puts back the clipboard text from before the selection was read, if it was changed.
fn restore(previous: Option<Option<String>>) -> Result<()> {
    match previous {
        Some(Some(text)) => set_private(text),
        // It held no text (e.g. an image); leave what is there.
        Some(None) | None => Ok(()),
    }
}
