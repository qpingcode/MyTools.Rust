//! Remember the normal window size and position, including across display layout changes.
use crate::{MainWindow, config, i18n};
use serde::{Deserialize, Serialize};
use slint::{
    ComponentHandle,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{cell::Cell, rc::Rc, time::Duration};

/// Saved sizes outside this physical range are treated as corrupt and replaced by the default.
const MIN_SANE_SIZE: u32 = 100;
const MAX_SANE_SIZE: u32 = 32_768;

/// Physical pixels; `width`/`height` are 0 in files written before sizes were remembered.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
struct Geometry {
    x: i32,
    y: i32,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

/// A display rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Monitor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub struct Tracker {
    timer: Rc<slint::Timer>,
    pending: Rc<Cell<Option<Geometry>>>,
    saved: Rc<Cell<Option<Geometry>>>,
}

impl Tracker {
    pub fn new(ui: &MainWindow) -> Self {
        let geometry = load();
        let pending = Rc::new(Cell::new(None));
        let saved = Rc::new(Cell::new(geometry));
        let timer = Rc::new(slint::Timer::default());
        let tracker = Self {
            timer: timer.clone(),
            pending: pending.clone(),
            saved: saved.clone(),
        };
        let mut restored = false;
        ui.window().on_winit_window_event(move |window, event| {
            if !restored {
                restored = window
                    .with_winit_window(|native| {
                        if let Some(geometry) = geometry {
                            restore(native, geometry);
                        }
                    })
                    .is_some();
                return EventResult::Propagate;
            }
            if matches!(
                event,
                winit::event::WindowEvent::Moved(_) | winit::event::WindowEvent::Resized(_)
            ) && let Some(current) = window.with_winit_window(normal_geometry).flatten()
            {
                pending.set(Some(current));
                let pending = pending.clone();
                let saved = saved.clone();
                timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(500),
                    move || flush(&pending, &saved),
                );
            }
            EventResult::Propagate
        });
        tracker
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.timer.stop();
        flush(&self.pending, &self.saved);
    }
}

fn restore(native: &winit::window::Window, saved: Geometry) {
    let size = native.inner_size();
    let Some(target) = placement(saved, (size.width, size.height), &monitors(native)) else {
        return;
    };
    if (target.width, target.height) != (size.width, size.height) {
        let _ =
            native.request_inner_size(winit::dpi::PhysicalSize::new(target.width, target.height));
    }
    native.set_outer_position(winit::dpi::PhysicalPosition::new(target.x, target.y));
}

/// Connected displays, primary first so it is the fallback for lost windows.
fn monitors(native: &winit::window::Window) -> Vec<Monitor> {
    let primary = native.primary_monitor();
    let mut handles: Vec<_> = native.available_monitors().collect();
    handles.sort_by_key(|handle| Some(handle) != primary.as_ref());
    handles
        .iter()
        .map(|handle| {
            let position = handle.position();
            let size = handle.size();
            Monitor {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect()
}

/// The current geometry, unless the window is hidden, minimized or maximized.
fn normal_geometry(native: &winit::window::Window) -> Option<Geometry> {
    let normal = native.is_visible().unwrap_or(false)
        && !native.is_minimized().unwrap_or(false)
        && !native.is_maximized();
    let position = native.outer_position().ok()?;
    let size = native.inner_size();
    // Windows parks minimized windows near (-32000, -32000).
    let parked = position.x <= -30_000 || position.y <= -30_000;
    (normal && !parked && sane_size(size.width, size.height)).then_some(Geometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

fn sane_size(width: u32, height: u32) -> bool {
    let range = MIN_SANE_SIZE..=MAX_SANE_SIZE;
    range.contains(&width) && range.contains(&height)
}

/// Where to put the window: the saved size (or `default_size` when it is missing or corrupt),
/// shrunk to fit its display and pulled fully onto it. A window that no longer touches any
/// display is centered on the first one. `None` when no display is known.
fn placement(saved: Geometry, default_size: (u32, u32), monitors: &[Monitor]) -> Option<Geometry> {
    let (width, height) = if sane_size(saved.width, saved.height) {
        (saved.width, saved.height)
    } else {
        default_size
    };
    let wanted = Geometry {
        width,
        height,
        ..saved
    };
    let best = monitors
        .iter()
        .map(|monitor| (overlap(wanted, *monitor), *monitor))
        .filter(|(area, _)| *area > 0)
        .max_by_key(|(area, _)| *area)
        .map(|(_, monitor)| monitor);
    let monitor = best.or_else(|| monitors.first().copied())?;
    let width = width.min(monitor.width);
    let height = height.min(monitor.height);
    let (x, y) = if best.is_some() {
        (
            clamp_axis(saved.x, width, monitor.x, monitor.width),
            clamp_axis(saved.y, height, monitor.y, monitor.height),
        )
    } else {
        (
            center_axis(width, monitor.x, monitor.width),
            center_axis(height, monitor.y, monitor.height),
        )
    };
    Some(Geometry {
        x,
        y,
        width,
        height,
    })
}

/// Visible area of the window on the display, in square physical pixels.
fn overlap(window: Geometry, monitor: Monitor) -> i64 {
    let span = |start: i32, length: u32, display: i32, display_length: u32| {
        let end = (i64::from(start) + i64::from(length))
            .min(i64::from(display) + i64::from(display_length));
        (end - i64::from(start).max(i64::from(display))).max(0)
    };
    span(window.x, window.width, monitor.x, monitor.width)
        * span(window.y, window.height, monitor.y, monitor.height)
}

/// Moves a span of `length` (no longer than the display) so it lies inside the display.
fn clamp_axis(start: i32, length: u32, display: i32, display_length: u32) -> i32 {
    let last = i64::from(display) + i64::from(display_length) - i64::from(length);
    i64::from(start).clamp(i64::from(display), last) as i32
}

fn center_axis(length: u32, display: i32, display_length: u32) -> i32 {
    (i64::from(display) + (i64::from(display_length) - i64::from(length)).max(0) / 2) as i32
}

fn path() -> anyhow::Result<std::path::PathBuf> {
    Ok(config::path()?.with_file_name("window-position.json"))
}

fn load() -> Option<Geometry> {
    let bytes = std::fs::read(path().ok()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save(geometry: Geometry) -> anyhow::Result<()> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec(&geometry)?)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    Ok(())
}

fn flush(pending: &Cell<Option<Geometry>>, saved: &Cell<Option<Geometry>>) {
    if let Some(geometry) = pending
        .take()
        .filter(|geometry| Some(*geometry) != saved.get())
    {
        match save(geometry) {
            Ok(()) => saved.set(Some(geometry)),
            Err(error) => eprintln!(
                "{}",
                i18n::format(
                    "window.geometry_save_failed",
                    &[("error", &error.to_string())]
                )
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: Monitor = Monitor {
        x: -1920,
        y: 0,
        width: 1920,
        height: 1080,
    };
    const MAIN: Monitor = Monitor {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    fn geometry(x: i32, y: i32, width: u32, height: u32) -> Geometry {
        Geometry {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn restores_saved_size_and_negative_display_positions() {
        let saved = geometry(-1200, 100, 1000, 700);
        assert_eq!(placement(saved, (780, 520), &[MAIN, LEFT]), Some(saved));
    }

    #[test]
    fn centers_windows_from_disconnected_displays_on_the_first_display() {
        let saved = geometry(-1200, 100, 860, 580);
        assert_eq!(
            placement(saved, (780, 520), &[MAIN]),
            Some(geometry(530, 250, 860, 580))
        );
        assert_eq!(
            placement(geometry(i32::MAX, i32::MAX, 860, 580), (780, 520), &[MAIN]),
            Some(geometry(530, 250, 860, 580))
        );
        assert_eq!(placement(saved, (780, 520), &[]), None);
    }

    #[test]
    fn pulls_partly_visible_windows_back_onto_their_display() {
        assert_eq!(
            placement(geometry(1500, 900, 800, 500), (780, 520), &[MAIN]),
            Some(geometry(1120, 580, 800, 500))
        );
        // Straddling two displays keeps the one holding most of the window.
        assert_eq!(
            placement(geometry(-300, -50, 800, 500), (780, 520), &[MAIN, LEFT]),
            Some(geometry(0, 0, 800, 500))
        );
    }

    #[test]
    fn shrinks_sizes_larger_than_the_display_after_a_resolution_change() {
        let small = Monitor {
            width: 1280,
            height: 720,
            ..MAIN
        };
        assert_eq!(
            placement(geometry(100, 100, 1600, 900), (780, 520), &[small]),
            Some(geometry(0, 0, 1280, 720))
        );
    }

    #[test]
    fn replaces_missing_or_corrupt_sizes_with_the_default() {
        for (width, height) in [(0, 0), (10, 600), (900, u32::MAX)] {
            assert_eq!(
                placement(geometry(200, 100, width, height), (780, 520), &[MAIN]),
                Some(geometry(200, 100, 780, 520))
            );
        }
    }

    #[test]
    fn reads_files_saved_before_sizes_were_remembered() {
        let old: Geometry = serde_json::from_str(r#"{"x":-1200,"y":100}"#).unwrap();
        assert_eq!(old, geometry(-1200, 100, 0, 0));
    }
}
