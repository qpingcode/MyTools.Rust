use crate::store::{Captured, Content, Entry, Store};
use anyhow::Result;
use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Text this plugin just put on the clipboard as plain text, and when: the change it causes
/// is not recorded again, since history already holds the formatted original.
static OWN_PLAIN_TEXT: Mutex<Option<(String, Instant)>> = Mutex::new(None);
/// How long a change caused by this plugin is expected to take to arrive.
const OWN_CHANGE_WINDOW: Duration = Duration::from_secs(2);

#[cfg(windows)]
#[path = "clipboard/windows.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "clipboard/macos.rs"]
mod platform;

pub fn start(store: Arc<Mutex<Store>>) {
    #[cfg(any(windows, target_os = "macos"))]
    {
        let (sender, changes) = std::sync::mpsc::channel();
        let watch_errors = store.clone();
        std::thread::spawn(move || {
            if let Err(error) = platform::watch(sender) {
                watch_errors
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .last_error = Some(error.to_string());
            }
        });
        std::thread::spawn(move || {
            for () in changes {
                for attempt in 0..8 {
                    match capture() {
                        Ok(Some(content)) if is_own_change(&content) => break,
                        Ok(Some(content)) => {
                            let mut state = store.lock().unwrap_or_else(|error| error.into_inner());
                            state.last_error =
                                state.add(content).err().map(|error| error.to_string());
                            break;
                        }
                        Ok(None) => break,
                        Err(error) => {
                            if attempt == 7 {
                                eprintln!("clipboard capture: {error:#}");
                                let mut state = store.lock().unwrap_or_else(|e| e.into_inner());
                                state.last_error = Some(crate::i18n::format(
                                    "capture_failed",
                                    &[("error", &format!("{error:#}"))],
                                ));
                                state.revision = state.revision.wrapping_add(1);
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            }
        });
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = store;
}

fn capture() -> Result<Option<Captured>> {
    #[cfg(any(windows, target_os = "macos"))]
    let sequence = platform::sequence();
    let content = read_content()?;
    #[cfg(any(windows, target_os = "macos"))]
    anyhow::ensure!(
        sequence == platform::sequence(),
        "Clipboard changed during capture"
    );
    Ok(content)
}

fn read_content() -> Result<Option<Captured>> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        if platform::excluded() {
            return Ok(None);
        }
        if let Some(files) = platform::files()? {
            return Ok(Some(Captured::Files(files)));
        }
    }
    let mut clipboard = arboard::Clipboard::new()?;
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => {
            let html = clipboard.get().html().ok();
            Ok(Some(Captured::Text { text, html }))
        }
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => read_image(&mut clipboard),
        Err(error) => Err(error.into()),
    }
}

fn read_image(clipboard: &mut arboard::Clipboard) -> Result<Option<Captured>> {
    match clipboard.get_image() {
        Ok(image) => Ok(Some(Captured::Image {
            width: image.width as u32,
            height: image.height as u32,
            rgba: image.bytes.into_owned(),
        })),
        Err(error) => {
            // Some screenshot tools expose CF_DIB directly; Windows' synthesized
            // DIBV5 representation is not always readable through arboard.
            #[cfg(windows)]
            if let Some(image) = platform::image()? {
                return Ok(Some(image));
            }
            match error {
                arboard::Error::ContentNotAvailable => Ok(None),
                error => Err(error.into()),
            }
        }
    }
}

/// Replaces what is on the clipboard with its text alone, dropping formatting such as HTML
/// or RTF, so the next paste inserts plain text.
pub fn keep_plain_text() -> Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    let text = match clipboard.get_text() {
        Ok(text) if !text.is_empty() => text,
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => {
            anyhow::bail!(crate::i18n::text("no_text"))
        }
        Err(error) => return Err(error.into()),
    };
    *OWN_PLAIN_TEXT
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some((text.clone(), Instant::now()));
    clipboard.set_text(text)?;
    Ok(())
}

/// Whether a captured change is the plain text [`keep_plain_text`] wrote; consumes it.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn is_own_change(content: &Captured) -> bool {
    let Captured::Text { text, html: None } = content else {
        return false;
    };
    let mut own = OWN_PLAIN_TEXT
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let matches = own
        .as_ref()
        .is_some_and(|(own, at)| own == text && at.elapsed() < OWN_CHANGE_WINDOW);
    if matches {
        *own = None;
    }
    matches
}

pub fn restore(entry: &Entry, root: &std::path::Path, plain: bool) -> Result<()> {
    match &entry.content {
        Content::Text { text, html } => {
            let mut clipboard = arboard::Clipboard::new()?;
            if !plain && let Some(html) = html {
                clipboard.set_html(html, Some(text))?;
            } else {
                clipboard.set_text(text)?;
            }
        }
        Content::Image { .. } => {
            let image = image::open(root.join(format!("{}.png", entry.id)))?.to_rgba8();
            arboard::Clipboard::new()?.set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: Cow::Owned(image.into_raw()),
            })?;
        }
        Content::Files { paths } => restore_files(paths)?,
    }
    Ok(())
}

fn restore_files(paths: &[PathBuf]) -> Result<()> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        platform::restore_files(paths)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = paths;
        anyhow::bail!(crate::i18n::text("unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_plain_text_this_plugin_wrote_is_skipped_once() {
        let plain = |text: &str| Captured::Text {
            text: text.into(),
            html: None,
        };
        *OWN_PLAIN_TEXT.lock().unwrap() = Some(("hello".into(), Instant::now()));
        let formatted = Captured::Text {
            text: "hello".into(),
            html: Some("<b>hello</b>".into()),
        };
        assert!(!is_own_change(&formatted));
        assert!(!is_own_change(&plain("other")));
        assert!(is_own_change(&plain("hello")));
        assert!(!is_own_change(&plain("hello")));
        *OWN_PLAIN_TEXT.lock().unwrap() = Some(("late".into(), Instant::now() - OWN_CHANGE_WINDOW));
        assert!(!is_own_change(&plain("late")));
    }
}
