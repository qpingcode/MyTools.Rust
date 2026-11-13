//! Environment variables such as API keys.

/// A variable's nonempty value. The inherited process environment comes first; Windows
/// launchers may still carry an older environment, so the persisted user and system
/// variables are read as well, and a key set in Windows settings works without a restart.
pub fn variable(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| persisted(name))
        .filter(|value| !value.trim().is_empty())
}

#[cfg(not(windows))]
fn persisted(_: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn persisted(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    read_variable(HKEY_CURRENT_USER, "Environment", name).or_else(|| {
        read_variable(
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            name,
        )
    })
}

#[cfg(windows)]
fn read_variable(
    root: windows_sys::Win32::System::Registry::HKEY,
    path: &str,
    name: &str,
) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW};
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes = 0;
    // Both strings are NUL-terminated. The first call only queries buffer size.
    let status = unsafe {
        RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != 0 || bytes == 0 || bytes > 64 * 1024 {
        return None;
    }
    let mut value = vec![0u16; (bytes as usize).div_ceil(2)];
    // The allocated buffer covers the byte count returned by the size query.
    let status = unsafe {
        RegGetValueW(
            root,
            path.as_ptr(),
            name.as_ptr(),
            flags,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    let end = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    String::from_utf16(&value[..end]).ok()
}
