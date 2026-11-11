//! IOKit power assertions, as `caffeinate -di` uses; they show in `pmset -g assertions` and
//! are released by the system when the process ends.
use std::ffi::{c_char, c_void};

type CFStringRef = *const c_void;
type IOPMAssertionID = u32;
const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
const LEVEL_ON: u32 = 255; // kIOPMAssertionLevelOn
const SUCCESS: i32 = 0; // kIOReturnSuccess

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        text: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFRelease(value: *const c_void);
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        kind: CFStringRef,
        level: u32,
        name: CFStringRef,
        id: *mut IOPMAssertionID,
    ) -> i32;
}

fn string(text: &std::ffi::CStr) -> CFStringRef {
    unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) }
}

fn assert(kind: &std::ffi::CStr) -> Result<(), String> {
    let (kind, name) = (string(kind), string(c"MyTools Keep Awake"));
    if kind.is_null() || name.is_null() {
        return Err("could not create the assertion name".into());
    }
    let mut id = 0;
    let result = unsafe { IOPMAssertionCreateWithName(kind, LEVEL_ON, name, &mut id) };
    unsafe {
        CFRelease(kind);
        CFRelease(name);
    }
    // The assertion is never released on purpose: it lasts until the process ends.
    if result == SUCCESS {
        Ok(())
    } else {
        Err(format!("IOPMAssertionCreateWithName returned {result:#x}"))
    }
}

pub fn keep_awake() -> Result<(), String> {
    assert(c"PreventUserIdleSystemSleep")?;
    assert(c"PreventUserIdleDisplaySleep")
}
