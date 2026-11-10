//! The bottom-right notice window: plugins that failed to start and startup problems such as
//! a shortcut that could not be registered. It never takes focus, stays above other windows
//! and closes when the user dismisses it, or after a while unless the pointer is over it.
use crate::{I18n, NoticeData, NoticeWindow, Theme, i18n, theme};
use mytools_core::Notice;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

/// How long notices stay up once the pointer is not over them.
const VISIBLE_FOR: Duration = Duration::from_secs(12);
/// Distance from the screen edges, in logical pixels.
const MARGIN: f32 = 16.0;
/// Older notices give way to newer ones beyond this many.
const MAX_SHOWN: usize = 5;

thread_local! {
    static CREATING: Cell<bool> = const { Cell::new(false) };
}

/// Whether the window being created is a notice, so the backend creates it without
/// activating it or giving it a taskbar button.
pub fn creating() -> bool {
    CREATING.with(Cell::get)
}

pub struct Notifier {
    window: RefCell<Option<NoticeWindow>>,
    shown: RefCell<Vec<Notice>>,
    close_at: Cell<Instant>,
    timer: slint::Timer,
    font: String,
}

impl Notifier {
    pub fn new(font: &str) -> Rc<Self> {
        Rc::new(Self {
            window: RefCell::new(None),
            shown: RefCell::new(Vec::new()),
            close_at: Cell::new(Instant::now()),
            timer: slint::Timer::default(),
            font: font.into(),
        })
    }

    /// Adds notices to the window, opening it if needed.
    pub fn show(self: &Rc<Self>, notices: Vec<Notice>) {
        if notices.is_empty() {
            return;
        }
        let rows: Vec<NoticeData> = {
            let mut shown = self.shown.borrow_mut();
            shown.extend(notices);
            let excess = shown.len().saturating_sub(MAX_SHOWN);
            shown.drain(..excess);
            shown
                .iter()
                .map(|notice| NoticeData {
                    title: notice.title.clone().into(),
                    message: notice.message.clone().into(),
                })
                .collect()
        };
        // A window shown before would take focus when shown again, so each appearance
        // creates a new one.
        if self.window.borrow().is_none() {
            match self.create() {
                Ok(window) => *self.window.borrow_mut() = Some(window),
                Err(_) => return,
            }
        }
        let window = self.window.borrow();
        let Some(window) = window.as_ref() else {
            return;
        };
        window.set_notices(ModelRc::new(VecModel::from(rows)));
        place(window);
        let _ = window.show();
        self.close_at.set(Instant::now() + VISIBLE_FOR);
        let notifier = Rc::downgrade(self);
        self.timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(500),
            move || {
                if let Some(notifier) = notifier.upgrade() {
                    notifier.expire();
                }
            },
        );
    }

    fn create(self: &Rc<Self>) -> Result<NoticeWindow, slint::PlatformError> {
        CREATING.with(|creating| creating.set(true));
        let window = NoticeWindow::new();
        CREATING.with(|creating| creating.set(false));
        let window = window?;
        theme::apply_configured(&window.global::<Theme>());
        window.global::<I18n>().on_text(|key| i18n::t(&key).into());
        window.set_app_font_family(self.font.clone().into());
        let weak = window.as_weak();
        window.on_content_resized(move || {
            if let Some(window) = weak.upgrade() {
                place(&window);
            }
        });
        let notifier = Rc::downgrade(self);
        window.on_dismiss(move || {
            if let Some(notifier) = notifier.upgrade() {
                notifier.close();
            }
        });
        Ok(window)
    }

    fn expire(&self) {
        let hovered = self
            .window
            .borrow()
            .as_ref()
            .is_some_and(NoticeWindow::get_hovered);
        if hovered {
            // Reading continues a little after the pointer leaves.
            self.close_at.set(Instant::now() + Duration::from_secs(3));
        } else if Instant::now() >= self.close_at.get() {
            self.close();
        }
    }

    fn close(&self) {
        self.timer.stop();
        self.shown.borrow_mut().clear();
        if let Some(window) = self.window.borrow_mut().take() {
            let _ = window.hide();
        }
    }
}

/// Puts the window in the bottom-right corner of the primary screen's work area.
fn place(window: &NoticeWindow) {
    let Some((left, top, width, height)) = work_area(window) else {
        return;
    };
    // From the layout rather than the native size, which lags behind a change of notices.
    let scale = window.window().scale_factor();
    let size = slint::LogicalSize::new(window.get_content_width(), window.get_content_height());
    let size = size.to_physical(scale);
    let margin = (MARGIN * scale) as i32;
    window.window().set_position(slint::PhysicalPosition::new(
        left + width - size.width as i32 - margin,
        top + height - size.height as i32 - margin,
    ));
}

/// The primary screen without the taskbar, in physical pixels: (left, top, width, height).
#[cfg(windows)]
fn work_area(_: &NoticeWindow) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{SPI_GETWORKAREA, SystemParametersInfoW},
    };
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&raw mut rect).cast(), 0) };
    (ok != 0).then_some((
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    ))
}

#[cfg(not(windows))]
fn work_area(window: &NoticeWindow) -> Option<(i32, i32, i32, i32)> {
    use slint::winit_030::WinitWindowAccessor;
    window
        .window()
        .with_winit_window(|native| {
            let monitor = native
                .primary_monitor()
                .or_else(|| native.current_monitor())?;
            let (position, size) = (monitor.position(), monitor.size());
            Some((
                position.x,
                position.y,
                size.width as i32,
                size.height as i32,
            ))
        })
        .flatten()
}
