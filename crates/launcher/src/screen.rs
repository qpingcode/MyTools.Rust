//! Where small windows such as notices and progress toasts go: a screen's work area (without
//! the taskbar), in physical pixels, with its scale factor.

pub struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
}

/// Which screen to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// The primary screen.
    Primary,
    /// The screen of the window in front, or the one under the pointer: where the user is
    /// looking.
    Focused,
}

#[cfg(windows)]
pub fn work_area(_: &slint::Window, screen: Screen) -> Option<WorkArea> {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
            MonitorFromPoint, MonitorFromWindow,
        },
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{GetCursorPos, GetForegroundWindow},
        },
    };
    let monitor = match screen {
        Screen::Primary => unsafe {
            MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
        },
        Screen::Focused => {
            let window = unsafe { GetForegroundWindow() };
            if window.is_null() {
                let mut cursor = POINT { x: 0, y: 0 };
                unsafe { GetCursorPos(&mut cursor) };
                unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) }
            } else {
                unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) }
            }
        }
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    let rect = info.rcWork;
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    let read = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let scale = if read >= 0 && dpi_x > 0 {
        dpi_x as f32 / 96.0
    } else {
        1.0
    };
    Some(WorkArea {
        left: rect.left,
        top: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        scale,
    })
}

#[cfg(not(windows))]
pub fn work_area(window: &slint::Window, screen: Screen) -> Option<WorkArea> {
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|native| {
            let monitor = match screen {
                Screen::Primary => native.primary_monitor(),
                Screen::Focused => None,
            }
            .or_else(|| native.current_monitor())
            .or_else(|| native.primary_monitor())?;
            let (position, size) = (monitor.position(), monitor.size());
            Some(WorkArea {
                left: position.x,
                top: position.y,
                width: size.width as i32,
                height: size.height as i32,
                scale: monitor.scale_factor() as f32,
            })
        })
        .flatten()
}
