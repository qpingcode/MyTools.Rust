//! Folders shown in open File Explorer windows, front window first.
use crate::win;
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IDispatch, IServiceProvider,
            },
            Variant::{VARIANT, VT_I4},
        },
        UI::{
            Shell::{
                IFolderView, IShellBrowser, IShellItem, IShellView, IShellWindows,
                SID_STopLevelBrowser, SIGDN_FILESYSPATH, ShellWindows,
            },
            WindowsAndMessaging::{GA_ROOT, GetAncestor, IsWindowVisible},
        },
    },
    core::Interface,
};

/// Empty when Explorer is not running or COM fails; the dialog still accepts typed paths.
pub fn open_folders() -> Vec<PathBuf> {
    let Ok(_com) = ComApartment::initialize() else {
        return Vec::new();
    };
    let Ok(windows) =
        (unsafe { CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_LOCAL_SERVER) })
    else {
        return Vec::new();
    };
    let order = win::z_order();
    let mut folders: Vec<(usize, bool, PathBuf)> = Vec::new();
    let count = unsafe { windows.Count() }.unwrap_or(0);
    for index in 0..count {
        // Windows can close while they are listed; skip those.
        let Some((root, visible, path)) = folder_of(&windows, index) else {
            continue;
        };
        let depth = order.iter().position(|&window| window == root);
        // Hidden tabs share the window of the visible one and come after it.
        folders.push((depth.unwrap_or(usize::MAX), !visible, path));
    }
    folders.sort_by_key(|(depth, hidden, _)| (*depth, *hidden));
    folders.into_iter().map(|(_, _, path)| path).collect()
}

fn folder_of(windows: &IShellWindows, index: i32) -> Option<(HWND, bool, PathBuf)> {
    unsafe {
        let view = windows
            .Item(&integer_variant(index))
            .ok()
            .and_then(shell_view)?;
        let view_window = view.GetWindow().ok()?;
        let folder: IFolderView = view.cast().ok()?;
        let item: IShellItem = folder.GetFolder().ok()?;
        // Virtual folders such as This PC have no file system path.
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        if name.is_null() {
            return None;
        }
        let path = PathBuf::from(OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0.cast()));
        Some((
            GetAncestor(view_window, GA_ROOT),
            IsWindowVisible(view_window).as_bool(),
            path,
        ))
    }
}

fn shell_view(window: IDispatch) -> Option<IShellView> {
    let provider: IServiceProvider = window.cast().ok()?;
    let browser: IShellBrowser = unsafe { provider.QueryService(&SID_STopLevelBrowser) }.ok()?;
    unsafe { browser.QueryActiveShellView() }.ok()
}

fn integer_variant(number: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    unsafe {
        let value = &mut *variant.Anonymous.Anonymous;
        value.vt = VT_I4;
        value.Anonymous.lVal = number;
    }
    variant
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> windows::core::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
