use crate::i18n;
use anyhow::{Result, ensure};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
use std::{
    ffi::c_void,
    sync::atomic::{AtomicI32, Ordering},
};

static TARGET: AtomicI32 = AtomicI32::new(0);
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGEventSourceFlagsState(state: i32) -> u64;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}

pub fn remember() {
    if let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() {
        let pid = app.processIdentifier();
        if pid != std::process::id() as i32 {
            TARGET.store(pid, Ordering::SeqCst);
        }
    }
}

pub fn validate() -> Result<()> {
    ensure!(unsafe { AXIsProcessTrusted() }, i18n::t("paste.permission"));
    ensure!(
        NSRunningApplication::runningApplicationWithProcessIdentifier(
            TARGET.load(Ordering::SeqCst)
        )
        .is_some(),
        i18n::t("paste.target_missing")
    );
    Ok(())
}

#[allow(deprecated)] // Available on supported macOS versions before activateFromApplication.
pub fn activate() -> Result<i32> {
    let pid = TARGET.load(Ordering::SeqCst);
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .ok_or_else(|| anyhow::anyhow!(i18n::t("paste.target_missing")))?;
    ensure!(
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps),
        i18n::t("paste.failed")
    );
    Ok(pid)
}

pub fn send(pid: i32) -> Result<()> {
    const MODIFIERS: u64 = (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20);
    for _ in 0..40 {
        if unsafe { CGEventSourceFlagsState(0) } & MODIFIERS == 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    ensure!(
        unsafe { CGEventSourceFlagsState(0) } & MODIFIERS == 0,
        i18n::t("paste.failed")
    );
    std::thread::sleep(std::time::Duration::from_millis(80));
    ensure!(
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .is_some_and(|app| app.processIdentifier() == pid),
        i18n::t("paste.failed")
    );
    for down in [true, false] {
        let event = unsafe { CGEventCreateKeyboardEvent(std::ptr::null(), 9, down) };
        ensure!(!event.is_null(), i18n::t("paste.failed"));
        unsafe {
            CGEventSetFlags(event, 1 << 20);
            CGEventPost(0, event);
            CFRelease(event);
        }
    }
    Ok(())
}
