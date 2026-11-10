//! Packaged (Store/MSIX) apps such as Claude have no Start Menu shortcut; Windows lists
//! them, with every other app, in the shell's AppsFolder.
use std::path::PathBuf;

/// One packaged app: its display name and `shell:AppsFolder\<AppUserModelID>` target.
pub struct App {
    pub name: String,
    pub path: PathBuf,
}

/// Lists packaged apps; desktop apps in the folder are left to the shortcut scan.
#[cfg(windows)]
pub fn apps() -> Vec<App> {
    list().unwrap_or_default()
}
#[cfg(not(windows))]
pub fn apps() -> Vec<App> {
    Vec::new()
}

#[cfg(windows)]
fn list() -> windows::core::Result<Vec<App>> {
    use windows::Win32::{
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
        UI::Shell::{
            BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
            SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
        },
    };
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }
    let name = |item: &IShellItem, kind: SIGDN| -> Option<String> {
        // SAFETY: the shell allocates the string; it is copied and freed exactly once.
        unsafe {
            let raw = item.GetDisplayName(kind).ok()?;
            let text = raw.to_string().ok();
            CoTaskMemFree(Some(raw.0 as *const _));
            text
        }
    };
    // SAFETY: plain COM calls on this scan thread; the `windows` wrappers check every result.
    unsafe {
        let _apartment = Apartment(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        let mut apps = Vec::new();
        loop {
            let mut item = [None];
            if items.Next(&mut item, None).is_err() {
                break;
            }
            let Some(item) = item[0].take() else { break };
            let (Some(id), Some(title)) = (
                name(&item, SIGDN_PARENTRELATIVEPARSING),
                name(&item, SIGDN_NORMALDISPLAY),
            ) else {
                continue;
            };
            if is_packaged_id(&id) && !title.trim().is_empty() {
                apps.push(App {
                    name: title,
                    path: mytools_plugin_api::paths::packaged_app(&id),
                });
            }
        }
        Ok(apps)
    }
}

/// Packaged AppUserModelIDs are `<PackageFamilyName>!<AppId>`; desktop entries are paths,
/// GUID-based known folders or plain IDs without `!`.
fn is_packaged_id(id: &str) -> bool {
    id.split_once('!').is_some_and(|(family, app)| {
        !family.is_empty() && !app.is_empty() && !family.contains(['\\', '/', ':'])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_only_package_family_app_ids() {
        assert!(is_packaged_id("Claude_pzs8sxrjxfjjc!Claude"));
        assert!(is_packaged_id(
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"
        ));
        assert!(!is_packaged_id(
            r"{6D809377-6AF0-444B-8957-A3773F02200E}\app.exe"
        ));
        assert!(!is_packaged_id("Microsoft.Windows.Explorer"));
        assert!(!is_packaged_id("!App"));
    }
}
