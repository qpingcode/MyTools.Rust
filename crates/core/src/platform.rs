use crate::i18n;
use anyhow::{Result, ensure};
use std::{ffi::OsStr, path::Path};

#[cfg(windows)]
#[path = "platform/elevation.rs"]
mod elevation;

pub fn can_run_as_admin(path: &Path) -> bool {
    cfg!(windows)
        && path.extension().is_some_and(|ext| {
            ["exe", "com", "lnk"]
                .iter()
                .any(|expected| ext.eq_ignore_ascii_case(expected))
        })
}

pub fn run_as_admin(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        elevation::run(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        anyhow::bail!(i18n::t("error.platform"))
    }
}

pub fn open_application(path: &Path) -> Result<()> {
    ensure!(
        path.exists(),
        "{}",
        i18n::format(
            "error.path_missing",
            &[("path", &path.display().to_string())]
        )
    );
    #[cfg(windows)]
    return open_target(&shell_path(path));
    #[cfg(not(windows))]
    open_target(path.as_os_str())
}

#[cfg(windows)]
fn shell_path(path: &Path) -> std::ffi::OsString {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    // Shell APIs need backslashes and ordinary drive/UNC paths, unlike filesystem APIs.
    let mut wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .map(|c| {
            if c == u16::from(b'/') {
                u16::from(b'\\')
            } else {
                c
            }
        })
        .collect();
    let verbatim: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    if wide.starts_with(&unc) {
        wide = "\\\\"
            .encode_utf16()
            .chain(wide.into_iter().skip(unc.len()))
            .collect();
    } else if wide.starts_with(&verbatim) {
        wide.drain(..verbatim.len());
    }
    std::ffi::OsString::from_wide(&wide)
}

pub fn open_url(url: &str) -> Result<()> {
    open_target(OsStr::new(url))
}

#[cfg(windows)]
fn open_target(target: &OsStr) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let target: Vec<u16> = target.encode_wide().chain(Some(0)).collect();
    let operation: Vec<u16> = "open\0".encode_utf16().collect();
    // Strings are null-terminated and live for the entire Win32 call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    ensure!(
        result as isize > 32,
        "{}",
        i18n::format(
            "error.open_windows",
            &[("code", &(result as isize).to_string())]
        )
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_target(target: &OsStr) -> Result<()> {
    let status = std::process::Command::new("open").arg(target).status()?;
    ensure!(status.success(), i18n::t("error.open_macos"));
    Ok(())
}

#[cfg(windows)]
pub fn reveal(path: &Path) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{SHOpenFolderAndSelectItems, SHParseDisplayName},
    };
    ensure!(
        path.exists(),
        "{}",
        i18n::format(
            "error.path_missing",
            &[("path", &path.display().to_string())]
        )
    );
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    // Called by the action worker, never the UI thread; COM ownership is balanced.
    ensure!(
        unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0,
        i18n::t("error.com_init")
    );
    let _apartment = Apartment;
    let path: Vec<u16> = shell_path(path).encode_wide().chain(Some(0)).collect();
    let mut item = ptr::null_mut();
    let parsed = unsafe {
        SHParseDisplayName(
            path.as_ptr(),
            ptr::null_mut(),
            &mut item,
            0,
            ptr::null_mut(),
        )
    };
    ensure!(
        parsed >= 0 && !item.is_null(),
        "{}",
        i18n::format("error.parse_windows", &[("code", &format!("{parsed:#x}"))])
    );
    let opened = unsafe { SHOpenFolderAndSelectItems(item, 0, ptr::null(), 0) };
    unsafe {
        CoTaskMemFree(item.cast());
    }
    ensure!(
        opened >= 0,
        "{}",
        i18n::format("error.reveal_windows", &[("code", &format!("{opened:#x}"))])
    );
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn reveal(path: &Path) -> Result<()> {
    ensure!(
        path.exists(),
        "{}",
        i18n::format(
            "error.path_missing",
            &[("path", &path.display().to_string())]
        )
    );
    let status = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .status()?;
    ensure!(status.success(), i18n::t("error.reveal_macos"));
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn open_target(_: &OsStr) -> Result<()> {
    anyhow::bail!(i18n::t("error.platform"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn reveal(_: &Path) -> Result<()> {
    anyhow::bail!(i18n::t("error.platform"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn normalizes_paths_for_windows_shell_without_losing_unicode() {
        assert_eq!(
            shell_path(Path::new("C:\\menu/程序\\app.lnk")),
            OsStr::new("C:\\menu\\程序\\app.lnk")
        );
        assert_eq!(
            shell_path(Path::new(r"\\?\D:\app.exe")),
            OsStr::new(r"D:\app.exe")
        );
        assert_eq!(
            shell_path(Path::new(r"\\?\UNC\server\share\文件.txt")),
            OsStr::new(r"\\server\share\文件.txt")
        );
    }
}
