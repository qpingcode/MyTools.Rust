use crate::{MainWindow, i18n};
use anyhow::Result;
use slint::ComponentHandle;

#[cfg(windows)]
#[path = "paste_windows.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "paste_macos.rs"]
mod platform;

pub fn remember_target() {
    #[cfg(any(windows, target_os = "macos"))]
    platform::remember();
}

/// Pastes into the application remembered by [`remember_target`] without touching the
/// launcher window, e.g. for a command run by its shortcut. Finishes once the paste
/// shortcut was sent.
pub async fn paste_into_target() -> Result<()> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        platform::validate()?;
        let target = platform::activate()?;
        let (sender, sent) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(platform::send(target));
        });
        sent.recv()
            .await
            .map_err(|_| anyhow::anyhow!(i18n::t("paste.failed")))?
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    anyhow::bail!(i18n::t("paste.failed"))
}

pub fn paste(ui: &MainWindow) -> Result<()> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        platform::validate()?;
        ui.hide()?;
        let target = match platform::activate() {
            Ok(target) => target,
            Err(error) => {
                let _ = ui.show();
                return Err(error);
            }
        };
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            if let Err(error) = platform::send(target) {
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_failed(true);
                    ui.set_status(
                        i18n::format("status.failed", &[("error", &error.to_string())]).into(),
                    );
                    let _ = ui.show();
                });
            }
        });
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = ui;
        anyhow::bail!(i18n::t("paste.failed"))
    }
}
