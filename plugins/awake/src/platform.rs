//! Asks the system not to sleep or turn the screen off while this process runs.

#[cfg(windows)]
#[path = "platform/windows.rs"]
mod imp;
#[cfg(target_os = "macos")]
#[path = "platform/macos.rs"]
mod imp;

/// Keeps the computer awake and the screen on until the process ends. Call it on a thread
/// that lives as long as the process.
pub fn keep_awake() -> Result<(), String> {
    #[cfg(any(windows, target_os = "macos"))]
    return imp::keep_awake();
    #[cfg(not(any(windows, target_os = "macos")))]
    Err("unsupported on this system".into())
}
