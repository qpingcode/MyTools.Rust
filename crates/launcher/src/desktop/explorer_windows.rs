use anyhow::Context;
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
                CSIDL_DESKTOP, IFolderView, IShellBrowser, IShellItemArray, IShellView,
                IShellWindows, SID_STopLevelBrowser, SIGDN_FILESYSPATH, SVGIO_SELECTION,
                SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
            },
            WindowsAndMessaging::{
                GA_ROOT, GetAncestor, GetClassNameW, GetShellWindow, GetWindowThreadProcessId,
                IsWindowVisible,
            },
        },
    },
    core::Interface,
};

pub fn selected_files(foreground: HWND, ambiguous: &str) -> anyhow::Result<Option<Vec<PathBuf>>> {
    let _com = ComApartment::initialize()?;
    let windows: IShellWindows =
        unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER) }
            .context("CoCreateInstance(ShellWindows)")?;
    let view = if is_desktop_window(foreground) {
        Some(desktop_view(&windows)?)
    } else {
        active_explorer_view(&windows, foreground, ambiguous)?
    };
    view.map(|view| selected_paths(&view))
        .transpose()
        .map_err(Into::into)
}

pub fn is_desktop_window(window: HWND) -> bool {
    let shell = unsafe { GetShellWindow() };
    if window.is_invalid() || shell.is_invalid() {
        return false;
    }
    if window == shell {
        return true;
    }
    let mut name = [0; 32];
    let len = unsafe { GetClassNameW(window, &mut name) }.max(0) as usize;
    if !matches!(
        String::from_utf16_lossy(&name[..len]).as_str(),
        "Progman" | "WorkerW"
    ) {
        return false;
    }
    let mut foreground_pid = 0;
    let mut shell_pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut foreground_pid));
        GetWindowThreadProcessId(shell, Some(&mut shell_pid));
    }
    shell_pid != 0 && shell_pid == foreground_pid
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

fn desktop_view(windows: &IShellWindows) -> windows::core::Result<IShellView> {
    let location = integer_variant(CSIDL_DESKTOP as i32);
    let mut window = 0;
    let dispatch = unsafe {
        windows.FindWindowSW(
            &location,
            &VARIANT::default(),
            SWC_DESKTOP,
            &mut window,
            SWFO_NEEDDISPATCH,
        )?
    };
    shell_view(dispatch)
}

fn shell_view(window: IDispatch) -> windows::core::Result<IShellView> {
    let provider: IServiceProvider = window.cast()?;
    let browser: IShellBrowser = unsafe { provider.QueryService(&SID_STopLevelBrowser)? };
    unsafe { browser.QueryActiveShellView() }
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

fn active_explorer_view(
    windows: &IShellWindows,
    foreground: HWND,
    ambiguous: &str,
) -> anyhow::Result<Option<IShellView>> {
    // COM is initialized on this thread. Explorer windows can close while enumerating.
    unsafe {
        let mut active = None;
        for index in 0..windows.Count()? {
            let Ok(view) = windows.Item(&integer_variant(index)).and_then(shell_view) else {
                continue;
            };
            let Ok(view_window) = view.GetWindow() else {
                continue;
            };
            // Tabs can share a top-level HWND. Ignore hidden views.
            if !IsWindowVisible(view_window).as_bool()
                || GetAncestor(view_window, GA_ROOT) != foreground
            {
                continue;
            }
            if let Some(previous) = &active {
                if previous == &view {
                    continue;
                }
                anyhow::bail!("{}", ambiguous);
            }
            active = Some(view);
        }
        Ok(active)
    }
}

fn selected_paths(view: &IShellView) -> windows::core::Result<Vec<PathBuf>> {
    unsafe {
        let folder: IFolderView = view.cast()?;
        if folder.ItemCount(SVGIO_SELECTION)? == 0 {
            return Ok(Vec::new());
        }
        let items: IShellItemArray = folder.Items(SVGIO_SELECTION)?;
        let mut paths = Vec::new();
        for index in 0..items.GetCount()? {
            let item = items.GetItemAt(index)?;
            // Virtual items such as This PC do not have a filesystem path.
            if let Ok(path) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                if !path.is_null() {
                    paths.push(OsString::from_wide(path.as_wide()).into());
                    CoTaskMemFree(Some(path.0.cast()));
                }
            }
        }
        Ok(paths)
    }
}
