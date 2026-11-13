//! A small "working on it" window for commands run by their shortcut, in the lower middle of
//! the screen the user is looking at. It appears only when the command takes a while, never
//! takes focus, and closes as soon as the command finishes.
use crate::{Theme, ToastWindow, notices, screen, theme};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, time::Duration};

/// Commands that finish sooner never show the toast.
const DELAY: Duration = Duration::from_millis(400);
/// The toast's bottom edge sits this share of the screen's height above its bottom.
const FROM_BOTTOM: f32 = 0.12;

pub struct Toast {
    window: RefCell<Option<ToastWindow>>,
    pending: slint::Timer,
    font: String,
}

impl Toast {
    pub fn new(font: &str) -> Rc<Self> {
        Rc::new(Self {
            window: RefCell::new(None),
            pending: slint::Timer::default(),
            font: font.into(),
        })
    }

    /// Shows `title` and `detail` unless [`Self::close`] is called within [`DELAY`].
    pub fn start(self: &Rc<Self>, title: String, detail: String) {
        self.close();
        let toast = Rc::downgrade(self);
        self.pending
            .start(slint::TimerMode::SingleShot, DELAY, move || {
                if let Some(toast) = toast.upgrade() {
                    toast.show(&title, &detail);
                }
            });
    }

    pub fn close(&self) {
        self.pending.stop();
        if let Some(window) = self.window.borrow_mut().take() {
            let _ = window.hide();
        }
    }

    fn show(&self, title: &str, detail: &str) {
        // A window shown before would take focus when shown again, so each appearance
        // creates a new one.
        let Ok(window) = notices::without_focus(ToastWindow::new) else {
            return;
        };
        theme::apply_configured(&window.global::<Theme>());
        window.set_app_font_family(self.font.clone().into());
        window.set_heading(title.into());
        window.set_detail(detail.into());
        let weak = window.as_weak();
        window.on_content_resized(move || {
            if let Some(window) = weak.upgrade() {
                place(&window);
            }
        });
        place(&window);
        let _ = window.show();
        // Once shown, the window has the screen's scale; settle it again.
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            if let Some(window) = weak.upgrade() {
                place(&window);
            }
        });
        *self.window.borrow_mut() = Some(window);
    }
}

/// Centers the window horizontally, a little above the bottom of the focused screen.
fn place(window: &ToastWindow) {
    let Some(area) = screen::work_area(window.window(), screen::Screen::Focused) else {
        return;
    };
    let size = slint::LogicalSize::new(window.get_content_width(), window.get_content_height())
        .to_physical(area.scale);
    let bottom = area.top + area.height - (area.height as f32 * FROM_BOTTOM) as i32;
    window.window().set_position(slint::PhysicalPosition::new(
        area.left + (area.width - size.width as i32) / 2,
        bottom - size.height as i32,
    ));
}
