//! Remember the normal window position, including across display layout changes.
use crate::{MainWindow, config, i18n};
use serde::{Deserialize, Serialize};
use slint::{
    ComponentHandle,
    winit_030::{EventResult, WinitWindowAccessor, winit},
};
use std::{cell::Cell, rc::Rc, time::Duration};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Position {
    x: i32,
    y: i32,
}

pub struct Tracker {
    timer: Rc<slint::Timer>,
    pending: Rc<Cell<Option<Position>>>,
    saved: Rc<Cell<Option<Position>>>,
}

impl Tracker {
    pub fn new(ui: &MainWindow) -> Self {
        let position = load();
        let pending = Rc::new(Cell::new(None));
        let saved = Rc::new(Cell::new(position));
        let timer = Rc::new(slint::Timer::default());
        let tracker = Self {
            timer: timer.clone(),
            pending: pending.clone(),
            saved: saved.clone(),
        };
        let mut restored = false;
        ui.window().on_winit_window_event(move |window, event| {
            if !restored {
                let initialized = window
                    .with_winit_window(|native| {
                        if let Some(position) = position {
                            let size = native.outer_size();
                            let monitors: Vec<_> = native.available_monitors().collect();
                            let bounds: Vec<_> = monitors
                                .iter()
                                .map(|monitor| {
                                    let p = monitor.position();
                                    let s = monitor.size();
                                    (p.x, p.y, s.width, s.height)
                                })
                                .collect();
                            if let Some(position) =
                                visible_position(position, (size.width, size.height), &bounds)
                            {
                                native.set_outer_position(winit::dpi::PhysicalPosition::new(
                                    position.x, position.y,
                                ));
                            }
                        }
                    })
                    .is_some();
                restored = initialized;
                return EventResult::Propagate;
            }
            if let winit::event::WindowEvent::Moved(position) = event {
                let normal = window
                    .with_winit_window(|native| {
                        native.is_visible().unwrap_or(false)
                            && !native.is_minimized().unwrap_or(false)
                            && !native.is_maximized()
                    })
                    .unwrap_or(false);
                if normal && position.x > -30_000 && position.y > -30_000 {
                    pending.set(Some(Position {
                        x: position.x,
                        y: position.y,
                    }));
                    let pending = pending.clone();
                    let saved = saved.clone();
                    timer.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(500),
                        move || flush(&pending, &saved),
                    );
                }
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

fn visible_position(
    position: Position,
    size: (u32, u32),
    monitors: &[(i32, i32, u32, u32)],
) -> Option<Position> {
    let (x, y) = (i64::from(position.x), i64::from(position.y));
    // Keep the whole window on a connected display; negative coordinates are valid.
    let monitor = monitors.iter().find(|&&(mx, my, width, height)| {
        x >= i64::from(mx)
            && y >= i64::from(my)
            && x + i64::from(size.0) <= i64::from(mx) + i64::from(width)
            && y + i64::from(size.1) <= i64::from(my) + i64::from(height)
    });
    if monitor.is_some() {
        return Some(position);
    }
    let &(mx, my, width, height) = monitors.first()?;
    Some(Position {
        x: (i64::from(mx) + (i64::from(width) - i64::from(size.0)).max(0) / 2) as i32,
        y: (i64::from(my) + (i64::from(height) - i64::from(size.1)).max(0) / 2) as i32,
    })
}

fn path() -> anyhow::Result<std::path::PathBuf> {
    Ok(config::path()?.with_file_name("window-position.json"))
}

fn load() -> Option<Position> {
    let bytes = std::fs::read(path().ok()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save(position: Position) -> anyhow::Result<()> {
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec(&position)?)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    Ok(())
}

fn flush(pending: &Cell<Option<Position>>, saved: &Cell<Option<Position>>) {
    if let Some(position) = pending
        .take()
        .filter(|position| Some(*position) != saved.get())
    {
        match save(position) {
            Ok(()) => saved.set(Some(position)),
            Err(error) => eprintln!(
                "{}",
                i18n::format(
                    "window.position_save_failed",
                    &[("error", &error.to_string())]
                )
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_negative_display_positions_and_recovers_disconnected_displays() {
        let position = Position { x: -1200, y: 100 };
        let monitors = [(-1920, 0, 1920, 1080), (0, 0, 1920, 1080)];
        assert_eq!(
            visible_position(position, (860, 580), &monitors),
            Some(position)
        );
        assert_eq!(
            visible_position(position, (860, 580), &monitors[1..]),
            Some(Position { x: 530, y: 250 })
        );
        assert_eq!(
            visible_position(
                Position {
                    x: i32::MAX,
                    y: i32::MAX
                },
                (860, 580),
                &monitors[1..]
            ),
            Some(Position { x: 530, y: 250 })
        );
        assert_eq!(visible_position(position, (860, 580), &[]), None);
    }
}
