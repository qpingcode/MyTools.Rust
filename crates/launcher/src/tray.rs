use crate::{AppTray, I18n, MainWindow, RestartDialog, autostart, config, desktop, i18n};
use slint::ComponentHandle;
use std::{cell::Cell, rc::Rc, time::Duration};

#[cfg(windows)]
#[path = "tray/windows_menu.rs"]
mod windows_menu;

pub struct Controller {
    restart: Rc<Cell<bool>>,
    _refresh: slint::Timer,
    _dialog: RestartDialog,
    #[cfg(windows)]
    _menu_icons: windows_menu::MenuIcons,
}

impl Controller {
    pub fn new(tray: &AppTray, ui: &MainWindow) -> anyhow::Result<Self> {
        let selected = config::load().unwrap_or_default().language;
        #[cfg(windows)]
        let menu_icons = windows_menu::MenuIcons::new(tray);
        let applied_language = selected.clone();
        tray.set_language(selected.into());
        let dialog = RestartDialog::new()?;
        dialog.global::<I18n>().on_text(|key| i18n::t(&key).into());
        crate::theme::apply_configured(&dialog.global::<crate::Theme>());
        dialog.set_app_font_family(ui.get_app_font_family());
        match autostart::enabled() {
            Ok(enabled) => tray.set_startup_enabled(enabled),
            Err(error) => show_error(ui, &error),
        }
        let restart = Rc::new(Cell::new(false));
        let weak = ui.as_weak();
        tray.on_open(move || {
            if let Some(ui) = weak.upgrade() {
                desktop::show(&ui);
            }
        });

        let weak = ui.as_weak();
        let weak_tray = tray.as_weak();
        let weak_dialog = dialog.as_weak();
        tray.on_select_language(move |language| {
            let (Some(ui), Some(tray)) = (weak.upgrade(), weak_tray.upgrade()) else {
                return;
            };
            if language == tray.get_language() && language.as_str() == applied_language {
                return;
            }
            match config::save_language(&language) {
                Ok(()) => {
                    tray.set_language(language.clone());
                    if let Some(dialog) = weak_dialog.upgrade() {
                        let result = if language.as_str() != applied_language {
                            dialog.show()
                        } else {
                            dialog.hide()
                        };
                        if let Err(error) = result {
                            show_error(&ui, &error.into());
                        }
                    }
                }
                Err(error) => show_error(&ui, &error),
            }
        });

        let restart_request = restart.clone();
        let weak_dialog = dialog.as_weak();
        dialog.on_response(move |now| {
            if let Some(dialog) = weak_dialog.upgrade() {
                let _ = dialog.hide();
            }
            if now {
                restart_request.set(true);
                let _ = slint::quit_event_loop();
            }
        });
        dialog
            .window()
            .on_close_requested(|| slint::CloseRequestResponse::HideWindow);

        let weak = ui.as_weak();
        let weak_tray = tray.as_weak();
        tray.on_toggle_startup(move |enabled| {
            let (Some(ui), Some(tray)) = (weak.upgrade(), weak_tray.upgrade()) else {
                return;
            };
            if let Err(error) = autostart::set(enabled) {
                show_error(&ui, &error);
            }
            match autostart::enabled() {
                Ok(actual) => tray.set_startup_enabled(actual),
                Err(error) => show_error(&ui, &error),
            }
        });
        tray.on_quit(|| {
            let _ = slint::quit_event_loop();
        });

        // Reflect changes made outside MyTools without writing system settings at startup.
        let weak_tray = tray.as_weak();
        let refresh = slint::Timer::default();
        refresh.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(3),
            move || {
                if let (Some(tray), Ok(enabled)) = (weak_tray.upgrade(), autostart::enabled()) {
                    tray.set_startup_enabled(enabled);
                }
            },
        );
        Ok(Self {
            restart,
            _refresh: refresh,
            _dialog: dialog,
            #[cfg(windows)]
            _menu_icons: menu_icons,
        })
    }

    pub fn restart_requested(&self) -> bool {
        self.restart.get()
    }
}

fn show_error(ui: &MainWindow, error: &anyhow::Error) {
    ui.set_failed(true);
    ui.set_status(i18n::format("settings.failed", &[("error", &format!("{error:#}"))]).into());
    desktop::show(ui);
}

/// Called only after releasing the hotkey, tray and search processes.
pub fn restart() -> anyhow::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    // The new process waits for this one to exit and release the single-instance lock.
    command.args(["--show", "--restart"]);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()?;
    Ok(())
}
