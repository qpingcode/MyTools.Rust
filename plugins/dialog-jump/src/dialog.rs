//! Finds the Save/Open dialog behind the launcher and moves it to another folder through its
//! address bar, the way a user would: Alt+D, type the path, Enter. The file name box is never
//! touched, and Enter is only sent once the address bar has the keyboard focus, so a failed
//! step can never press the dialog's Save or Open button.
use crate::win;
use std::{
    path::Path,
    thread::sleep,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            SendInput, VIRTUAL_KEY, VK_CONTROL, VK_D, VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN,
            VK_SHIFT,
        },
        WindowsAndMessaging::{
            GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, IsChild, IsWindow,
            SendMessageW, SetForegroundWindow, WM_SETTEXT,
        },
    },
};

/// The control id of the address bar's edit box (and its combo box) in the shell's file
/// dialogs; File Explorer uses the same one.
const ADDRESS_ID: i32 = 41477;
const POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug)]
pub struct Dialog(HWND);

// A window handle is a plain identifier, valid in every thread.
unsafe impl Send for Dialog {}

impl Dialog {
    pub fn title(&self) -> String {
        win::title(self.0)
    }
}

/// A Save/Open dialog of the shell (`IFileDialog`, or `GetOpenFileName` upgraded to it):
/// a `#32770` window with a folder view and an address bar. The address bar's edit box
/// only exists once it was used, so look for the bar itself.
fn is_file_dialog(window: HWND) -> bool {
    if win::class_name(window) != "#32770" {
        return false;
    }
    let children = win::descendants(window);
    let has_view = children
        .iter()
        .any(|&child| win::class_name(child) == "SHELLDLL_DefView");
    has_view
        && children
            .iter()
            .any(|&child| win::class_name(child) == "Address Band Root")
}

fn is_address_edit(window: HWND) -> bool {
    win::control_id(window) == ADDRESS_ID && win::class_name(window) == "Edit"
}

/// The file dialog the user came from: the front window, or, while the launcher is in front,
/// the first ordinary window behind it.
pub fn behind_launcher() -> Option<Dialog> {
    let front = unsafe { GetForegroundWindow() };
    if is_file_dialog(front) {
        return Some(Dialog(front));
    }
    let launcher = win::process_id(front);
    win::z_order()
        .into_iter()
        .find(|&window| win::process_id(window) != launcher && win::is_ordinary(window))
        .filter(|&window| is_file_dialog(window))
        .map(Dialog)
}

/// Waits for the dialog to come back to the front after the launcher hides, then jumps.
pub fn jump(dialog: Dialog, folder: &Path) -> Result<(), String> {
    let window = dialog.0;
    wait_until(Duration::from_secs(1), || in_front(window))
        .or_else(|_| {
            unsafe {
                let _ = SetForegroundWindow(window);
            }
            wait_until(Duration::from_secs(1), || in_front(window))
        })
        .map_err(|_| "the dialog did not come to the front".to_owned())?;
    wait_until(Duration::from_secs(2), modifiers_released)
        .map_err(|_| "modifier keys are still held".to_owned())?;
    press(&[VK_MENU, VK_D])?;
    let address = wait_for(Duration::from_millis(600), || focused_address(window))
        .ok_or("Alt+D did not focus the address bar")?;
    let path = mytools_plugin_api::paths::display(folder);
    enter_path(address, &path)?;
    if !in_front(window) || focused_address(window) != Some(address) {
        return Err("the focus left the address bar".into());
    }
    press(&[VK_RETURN])
}

fn in_front(window: HWND) -> bool {
    unsafe { IsWindow(Some(window)) }.as_bool() && unsafe { GetForegroundWindow() } == window
}

/// The address bar's edit box when it has the keyboard focus in `dialog`.
fn focused_address(dialog: HWND) -> Option<HWND> {
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(win::thread_id(dialog), &mut info) }.ok()?;
    let focus = info.hwndFocus;
    (!focus.is_invalid()
        && unsafe { IsChild(dialog, focus) }.as_bool()
        && is_address_edit(focus))
    .then_some(focus)
}

/// Puts the path in the address bar and waits until it stays there: right after Alt+D the
/// shell may still fill the box with the current location and overwrite it, so the path is
/// set again until it survives a few polls in a row.
fn enter_path(address: HWND, path: &str) -> Result<(), String> {
    const STABLE_POLLS: u32 = 3;
    let mut stable = 0;
    let settled = wait_until(Duration::from_millis(800), || {
        if win::control_text(address) == path {
            stable += 1;
        } else {
            stable = 0;
            set_text(address, path);
        }
        stable >= STABLE_POLLS
    });
    settled.map_err(|()| {
        let shown = win::control_text(address);
        format!("the address bar did not keep the path {path:?}, it shows {shown:?}")
    })
}

fn set_text(control: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        SendMessageW(
            control,
            WM_SETTEXT,
            None,
            Some(LPARAM(wide.as_ptr() as isize)),
        );
    }
}

fn modifiers_released() -> bool {
    [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .all(|key| unsafe { GetAsyncKeyState(key.0 as i32) } >= 0)
}

/// Presses the keys in order and releases them in reverse, e.g. Alt+D.
fn press(keys: &[VIRTUAL_KEY]) -> Result<(), String> {
    let input = |key: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        },
    };
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&key| input(key, false))
        .chain(keys.iter().rev().map(|&key| input(key, true)))
        .collect();
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("SendInput was blocked".into())
    }
}

fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) -> Result<(), ()> {
    wait_for(timeout, || ready().then_some(())).ok_or(())
}

fn wait_for<T>(timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        sleep(POLL);
    }
}
