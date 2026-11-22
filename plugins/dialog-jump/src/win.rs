//! Small Win32 window helpers shared by dialog detection and the Explorer folder list.
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
        UI::WindowsAndMessaging::{
            EnumChildWindows, EnumWindows, GWL_EXSTYLE, GetClassNameW, GetDlgCtrlID,
            GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
            SendMessageW, WM_GETTEXT, WM_GETTEXTLENGTH, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
        },
    },
    core::BOOL,
};

pub fn class_name(window: HWND) -> String {
    let mut name = [0u16; 64];
    let len = unsafe { GetClassNameW(window, &mut name) }.max(0) as usize;
    String::from_utf16_lossy(&name[..len])
}

pub fn title(window: HWND) -> String {
    let mut text = [0u16; 256];
    let len = unsafe { GetWindowTextW(window, &mut text) }.max(0) as usize;
    String::from_utf16_lossy(&text[..len])
}

/// The text of a control in another process, read with `WM_GETTEXT`.
pub fn control_text(control: HWND) -> String {
    let len = unsafe { SendMessageW(control, WM_GETTEXTLENGTH, None, None) }.0.max(0) as usize;
    let mut text = vec![0u16; len + 1];
    let copied = unsafe {
        SendMessageW(
            control,
            WM_GETTEXT,
            Some(WPARAM(text.len())),
            Some(LPARAM(text.as_mut_ptr() as isize)),
        )
    }
    .0
    .clamp(0, len as isize) as usize;
    String::from_utf16_lossy(&text[..copied])
}

pub fn process_id(window: HWND) -> u32 {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    pid
}

pub fn thread_id(window: HWND) -> u32 {
    unsafe { GetWindowThreadProcessId(window, None) }
}

pub fn control_id(window: HWND) -> i32 {
    unsafe { GetDlgCtrlID(window) }
}

/// A window the user can see and switch to: visible, not cloaked (e.g. a suspended Store
/// app), and neither a tool window nor always on top like the taskbar.
pub fn is_ordinary(window: HWND) -> bool {
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return false;
    }
    let style = unsafe { GetWindowLongW(window, GWL_EXSTYLE) } as u32;
    if style & (WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) != 0 {
        return false;
    }
    let mut cloaked = 0u32;
    let read = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
    };
    read.is_err() || cloaked == 0
}

/// Top-level windows, front to back.
pub fn z_order() -> Vec<HWND> {
    let mut windows: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(&mut windows as *mut Vec<HWND> as isize),
        );
    }
    windows
}

/// All descendants of `parent`.
pub fn descendants(parent: HWND) -> Vec<HWND> {
    let mut windows: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumChildWindows(
            Some(parent),
            Some(collect),
            LPARAM(&mut windows as *mut Vec<HWND> as isize),
        );
    }
    windows
}

unsafe extern "system" fn collect(window: HWND, list: LPARAM) -> BOOL {
    let list = unsafe { &mut *(list.0 as *mut Vec<HWND>) };
    list.push(window);
    true.into()
}
