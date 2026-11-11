use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

/// Like PowerToys Awake: the request lasts (`ES_CONTINUOUS`) until the calling thread ends,
/// and shows in `powercfg /requests` under SYSTEM and DISPLAY.
pub fn keep_awake() -> Result<(), String> {
    let previous = unsafe {
        SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)
    };
    if previous == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
