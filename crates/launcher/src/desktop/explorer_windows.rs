use anyhow::Context;
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IServiceProvider,
            },
            Variant::{VARIANT, VT_I4},
        },
        UI::{
            Shell::{
                IFolderView, IShellBrowser, IShellItemArray, IShellView, IShellWindows,
                SID_STopLevelBrowser, SIGDN_FILESYSPATH, SVGIO_SELECTION, ShellWindows,
            },
            WindowsAndMessaging::{GA_ROOT, GetAncestor, IsWindowVisible},
        },
    },
    core::Interface,
};

pub fn selected_files(foreground: HWND, ambiguous: &str) -> anyhow::Result<Option<Vec<PathBuf>>> {
    let _com = ComApartment::initialize()?;
    active_explorer_view(foreground, ambiguous)?
        .map(|view| selected_paths(&view))
        .transpose()
        .map_err(Into::into)
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

fn active_explorer_view(foreground: HWND, ambiguous: &str) -> anyhow::Result<Option<IShellView>> {
    // COM is initialized on this thread. Explorer windows can close while enumerating.
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)
            .context("CoCreateInstance(ShellWindows)")?;
        let mut active = None;
        for index in 0..windows.Count()? {
            let mut variant = VARIANT::default();
            let value = &mut *variant.Anonymous.Anonymous;
            value.vt = VT_I4;
            value.Anonymous.lVal = index;
            let Ok(view) = windows
                .Item(&variant)
                .and_then(|window| window.cast::<IServiceProvider>())
                .and_then(|provider| provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser))
                .and_then(|browser| browser.QueryActiveShellView())
            else {
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
