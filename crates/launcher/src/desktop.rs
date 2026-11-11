use crate::{MainWindow, i18n};
use anyhow::{Context, Result};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use mytools_core::{Action, HotkeyBinding, HotkeyTarget};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::sync::{Arc, Mutex};

type Routes = Arc<Mutex<Vec<(u32, HotkeyTarget)>>>;

/// The manager is owned by the UI thread on both Windows and macOS.
pub struct Desktop {
    manager: GlobalHotKeyManager,
    hotkey: HotKey,
    /// The plugin and command shortcuts registered now, and what each one does.
    bound: Vec<HotKey>,
    routes: Routes,
}

pub fn configure_backend() -> Result<()> {
    let backend = slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attributes| {
            if !crate::notices::creating() {
                return attributes;
            }
            // Notices appear without stealing focus or adding a taskbar button.
            let attributes = attributes.with_active(false);
            #[cfg(windows)]
            let attributes = {
                use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
                attributes.with_skip_taskbar(true)
            };
            attributes
        });
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
    /// Registers Alt+Space; plugin shortcuts follow with [`Desktop::bind`] once the
    /// catalog has loaded. `run_command` receives command shortcuts, on any thread.
    pub fn new(
        ui: &MainWindow,
        run_command: impl Fn(Action) + Send + Sync + 'static,
    ) -> Result<Self> {
        use mytools_plugin_api::log;
        let manager = GlobalHotKeyManager::new()
            .inspect_err(|error| {
                log::error(
                    "hotkey.failed",
                    format!("global shortcuts unavailable: {error}"),
                )
            })
            .context(i18n::t("desktop.init_failed"))?;
        let hotkey = HotKey::new(Some(Modifiers::ALT), Code::Space);
        let weak = ui.as_weak();
        manager
            .register(hotkey)
            .inspect_err(|error| {
                log::error(
                    "hotkey.failed",
                    format!("could not register Alt+Space: {error}"),
                )
            })
            .context(i18n::t("desktop.register_failed"))?;
        let routes: Routes = Arc::default();
        let handler_routes = routes.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            if event.id == hotkey.id() {
                let _ = weak.upgrade_in_event_loop(|ui| {
                    show(&ui);
                    ui.invoke_focus_search_select_all();
                });
                return;
            }
            let target = handler_routes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .find(|(id, _)| *id == event.id)
                .map(|(_, target)| target.clone());
            match target {
                Some(HotkeyTarget::Scope(query)) => {
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        show(&ui);
                        ui.set_query(query.clone().into());
                        ui.invoke_search(query.into());
                        ui.invoke_focus_search_at_end();
                    });
                }
                Some(HotkeyTarget::Command(action)) => run_command(action),
                None => {}
            }
        }));
        Ok(Self {
            manager,
            hotkey,
            bound: Vec::new(),
            routes,
        })
    }

    /// Replaces the plugin and command shortcuts, e.g. after the settings changed. Returns
    /// one message per shortcut that could not be registered; the others still work.
    pub fn bind(&mut self, bindings: Vec<HotkeyBinding>) -> Vec<String> {
        for hotkey in self.bound.drain(..) {
            let _ = self.manager.unregister(hotkey);
        }
        let mut routes = Vec::new();
        let mut errors = Vec::new();
        for binding in bindings {
            match self.register(&binding.hotkey) {
                Ok(key) => {
                    routes.push((key.id(), binding.target));
                    self.bound.push(key);
                }
                Err(error) => {
                    mytools_plugin_api::log::warn(
                        "hotkey.failed",
                        format!(
                            "could not register {} for {}: {error}",
                            binding.hotkey, binding.owner
                        ),
                    );
                    errors.push(i18n::format(
                        "desktop.plugin_hotkey_failed",
                        &[("plugin", &binding.owner), ("error", &error.to_string())],
                    ));
                }
            }
        }
        *self
            .routes
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = routes;
        errors
    }

    fn register(&self, text: &str) -> Result<HotKey> {
        let parsed = mytools_plugin_api::hotkeys::Hotkey::parse(text)
            .with_context(|| i18n::format("desktop.hotkey_invalid", &[("hotkey", text)]))?;
        let key: HotKey = parsed.registration().parse()?;
        anyhow::ensure!(
            key.id() != self.hotkey.id() && !self.bound.iter().any(|bound| bound.id() == key.id()),
            i18n::format("desktop.hotkey_taken", &[("hotkey", &parsed.to_string())])
        );
        self.manager.register(key)?;
        Ok(key)
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = self.manager.unregister(self.hotkey);
        for hotkey in &self.bound {
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
    #[cfg(windows)]
    set_taskbar_icon(ui);
    ui.window().set_minimized(false);
    ui.invoke_focus_search();
    ui.window()
        .with_winit_window(|window| window.focus_window());
}

/// Slint applies `Window.icon` before the native window exists, which on Windows sets only
/// the small (title bar / Alt+Tab) icon; the taskbar shows the big one. Load it from the icon
/// resource that build.rs embeds.
#[cfg(windows)]
fn set_taskbar_icon(ui: &MainWindow) {
    use slint::winit_030::winit::{
        platform::windows::{IconExtWindows, WindowExtWindows},
        window::Icon,
    };
    ui.window().with_winit_window(|window| {
        if let Ok(icon) = Icon::from_resource(1, None) {
            window.set_taskbar_icon(Some(icon));
        }
    });
}

#[path = "desktop/drag.rs"]
pub mod drag;
#[path = "desktop/paste.rs"]
mod paste;
pub use paste::{paste, paste_into_target, remember_target};

pub fn drag(ui: &MainWindow) {
    ui.window().with_winit_window(|window| {
        let _ = window.drag_window();
    });
}

/// Starts a native resize from a frameless-window edge, e.g. `"se"` for the bottom-right corner.
pub fn drag_resize(ui: &MainWindow, edge: &str) {
    use slint::winit_030::winit::window::ResizeDirection;
    let direction = match edge {
        "n" => ResizeDirection::North,
        "s" => ResizeDirection::South,
        "e" => ResizeDirection::East,
        "w" => ResizeDirection::West,
        "ne" => ResizeDirection::NorthEast,
        "nw" => ResizeDirection::NorthWest,
        "se" => ResizeDirection::SouthEast,
        "sw" => ResizeDirection::SouthWest,
        _ => return,
    };
    ui.window().with_winit_window(|window| {
        let _ = window.drag_resize_window(direction);
    });
}
