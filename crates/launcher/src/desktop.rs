use crate::{MainWindow, i18n};
use anyhow::{Context, Result};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};

/// The manager is owned by the UI thread on both Windows and macOS.
pub struct Desktop {
    manager: GlobalHotKeyManager,
    hotkey: HotKey,
    plugin_hotkeys: Vec<HotKey>,
    pub registration_errors: Vec<String>,
}

pub fn configure_backend() -> Result<()> {
    let backend = slint::BackendSelector::new().backend_name("winit".into());
    #[cfg(target_os = "macos")]
    let backend = {
        use slint::winit_030::winit::{
            event_loop::EventLoop,
            platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS},
        };
        let mut builder = EventLoop::with_user_event();
        builder.with_activation_policy(ActivationPolicy::Accessory);
        backend.with_winit_event_loop_builder(builder)
    };
    backend.select()?;
    Ok(())
}

impl Desktop {
    pub fn new(
        ui: &MainWindow,
        overrides: &std::collections::HashMap<String, String>,
    ) -> Result<Self> {
        let manager = GlobalHotKeyManager::new().context(i18n::t("desktop.init_failed"))?;
        let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Space);
        let weak = ui.as_weak();
        manager
            .register(hotkey)
            .context(i18n::t("desktop.register_failed"))?;
        let mut plugin_hotkeys = Vec::new();
        let mut routes = Vec::new();
        let mut registration_errors = Vec::new();
        if let Ok(shortcuts) = mytools_core::activation_shortcuts() {
            for shortcut in shortcuts {
                let text = overrides
                    .get(&shortcut.plugin_id)
                    .unwrap_or(&shortcut.hotkey);
                if text.is_empty() {
                    continue;
                }
                let result = text
                    .parse::<HotKey>()
                    .map_err(anyhow::Error::from)
                    .and_then(|key| {
                        manager.register(key)?;
                        routes.push((key.id(), shortcut.query.clone()));
                        plugin_hotkeys.push(key);
                        Ok(())
                    });
                if let Err(error) = result {
                    registration_errors.push(i18n::format(
                        "desktop.plugin_hotkey_failed",
                        &[
                            ("plugin", &shortcut.plugin_id),
                            ("error", &error.to_string()),
                        ],
                    ));
                    ui.set_failed(true);
                    ui.set_status(
                        i18n::format(
                            "desktop.plugin_hotkey_failed",
                            &[
                                ("plugin", &shortcut.plugin_id),
                                ("error", &error.to_string()),
                            ],
                        )
                        .into(),
                    );
                }
            }
        }
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            if event.id == hotkey.id() && event.state == HotKeyState::Pressed {
                let _ = weak.upgrade_in_event_loop(|ui| {
                    show(&ui);
                    ui.invoke_focus_search_select_all();
                });
            } else if let Some((_, query)) = routes.iter().find(|(id, _)| *id == event.id) {
                let query = query.clone();
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    show(&ui);
                    ui.set_query(query.clone().into());
                    ui.invoke_search(query.into());
                    ui.invoke_focus_search_at_end();
                });
            }
        }));
        Ok(Self {
            manager,
            hotkey,
            plugin_hotkeys,
            registration_errors,
        })
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = self.manager.unregister(self.hotkey);
        for hotkey in &self.plugin_hotkeys {
            let _ = self.manager.unregister(*hotkey);
        }
    }
}

pub fn show(ui: &MainWindow) {
    paste::remember_target();
    if let Err(error) = ui.show() {
        ui.set_failed(true);
        ui.set_status(i18n::format("desktop.show_failed", &[("error", &error.to_string())]).into());
        return;
    }
    ui.window().set_minimized(false);
    ui.invoke_focus_search();
    ui.window()
        .with_winit_window(|window| window.focus_window());
}

#[path = "desktop/paste.rs"]
mod paste;
pub use paste::paste;

pub fn drag(ui: &MainWindow) {
    ui.window().with_winit_window(|window| {
        let _ = window.drag_window();
    });
}
