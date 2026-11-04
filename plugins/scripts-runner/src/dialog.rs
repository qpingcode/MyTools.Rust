//! Native folder picker for the working directory.
use std::path::PathBuf;

/// Shows the Windows folder picker, starting in `start` when it is an existing folder.
/// Returns `None` when the user cancels or the dialog cannot be shown.
#[cfg(windows)]
pub fn pick_folder(start: &str) -> Option<PathBuf> {
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            UI::{
                Input::KeyboardAndMouse::GetActiveWindow,
                Shell::{
                    FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog,
                    IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
                },
            },
        },
        core::HSTRING,
    };
    // SAFETY: plain COM calls on the UI thread; every returned pointer is checked by the
    // `windows` wrappers and the display-name buffer is freed exactly once.
    unsafe {
        // S_OK and S_FALSE (already initialised) both need a matching CoUninitialize;
        // RPC_E_CHANGED_MODE means another mode owns the thread and must not be undone.
        let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let picked = (|| {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let options = dialog.GetOptions().ok()?;
            dialog
                .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
                .ok()?;
            let start = start.trim();
            if !start.is_empty()
                && std::path::Path::new(start).is_dir()
                && let Ok(folder) =
                    SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start), None)
            {
                let _ = dialog.SetFolder(&folder);
            }
            // Owned by the manager window so it stays modal and on top of it.
            let owner = GetActiveWindow();
            dialog.Show((!owner.is_invalid()).then_some(owner)).ok()?;
            let name = dialog
                .GetResult()
                .ok()?
                .GetDisplayName(SIGDN_FILESYSPATH)
                .ok()?;
            let path = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as *const _));
            path.map(PathBuf::from)
        })();
        if initialised {
            CoUninitialize();
        }
        picked
    }
}

#[cfg(not(windows))]
pub fn pick_folder(_start: &str) -> Option<PathBuf> {
    None
}
