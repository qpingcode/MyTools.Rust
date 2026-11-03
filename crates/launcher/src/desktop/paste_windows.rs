use crate::i18n;
use anyhow::{Result, ensure};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            SendInput, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
        },
    },
};

static TARGET: AtomicIsize = AtomicIsize::new(0);
static PID: AtomicU32 = AtomicU32::new(0);

pub fn remember() {
    let window = unsafe { GetForegroundWindow() };
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    if !window.is_null() && pid != 0 && pid != std::process::id() {
        TARGET.store(window as isize, Ordering::SeqCst);
        PID.store(pid, Ordering::SeqCst);
    }
}

pub fn validate() -> Result<()> {
    let window = TARGET.load(Ordering::SeqCst) as HWND;
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    ensure!(
        unsafe { IsWindow(window) } != 0 && pid != 0 && pid == PID.load(Ordering::SeqCst),
        i18n::t("paste.target_missing")
    );
    Ok(())
}

pub fn activate() -> Result<isize> {
    validate()?;
    let window = TARGET.load(Ordering::SeqCst);
    unsafe {
        SetForegroundWindow(window as HWND);
    }
    Ok(window)
}

pub fn send(target: isize) -> Result<()> {
    for _ in 0..40 {
        if [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .all(|key| unsafe { GetAsyncKeyState(*key as i32) } >= 0)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    ensure!(
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .all(|key| unsafe { GetAsyncKeyState(*key as i32) } >= 0),
        i18n::t("paste.failed")
    );
    std::thread::sleep(std::time::Duration::from_millis(80));
    ensure!(
        unsafe { GetForegroundWindow() } as isize == target,
        i18n::t("paste.failed")
    );
    fn key(code: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: code,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    ..unsafe { std::mem::zeroed() }
                },
            },
        }
    }
    let inputs = [
        key(VK_CONTROL, false),
        key(b'V' as u16, false),
        key(b'V' as u16, true),
        key(VK_CONTROL, true),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        let releases = [key(b'V' as u16, true), key(VK_CONTROL, true)];
        unsafe {
            SendInput(
                releases.len() as u32,
                releases.as_ptr(),
                size_of::<INPUT>() as i32,
            );
        }
        anyhow::bail!(i18n::t("paste.failed"));
    }
    Ok(())
}
