use std::path::{Path, PathBuf};

/// Shell prefix of an installed packaged (Store/MSIX) app, followed by its AppUserModelID.
const APPS_FOLDER: &str = r"shell:AppsFolder\";

/// The launch target of a packaged app, e.g. `shell:AppsFolder\Claude_pzs8sxrjxfjjc!Claude`.
pub fn packaged_app(app_id: &str) -> PathBuf {
    PathBuf::from(format!("{APPS_FOLDER}{app_id}"))
}

/// Whether a path names a packaged app rather than a file. Such targets have no file to
/// check, copy or reveal; only the shell can open them.
pub fn is_packaged_app(path: &Path) -> bool {
    path.to_str().is_some_and(|text| {
        text.len() > APPS_FOLDER.len()
            && text.is_char_boundary(APPS_FOLDER.len())
            && text[..APPS_FOLDER.len()].eq_ignore_ascii_case(APPS_FOLDER)
            && !text.contains('\0')
    })
}

/// Format paths for display/copy without changing the path used for filesystem IO.
pub fn display(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
        match prefix.kind() {
            std::path::Prefix::VerbatimDisk(_) => {
                if let Some(path) = text.strip_prefix(r"\\?\") {
                    return path.to_owned();
                }
            }
            std::path::Prefix::VerbatimUNC(..) => {
                if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
                    return format!(r"\\{path}");
                }
            }
            _ => {}
        }
    }
    text.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognises_packaged_app_targets_only() {
        assert!(is_packaged_app(&packaged_app(
            "Claude_pzs8sxrjxfjjc!Claude"
        )));
        assert!(is_packaged_app(Path::new(r"SHELL:appsfolder\x!y")));
        assert!(!is_packaged_app(Path::new(r"shell:AppsFolder\")));
        assert!(!is_packaged_app(Path::new(r"C:\Apps\Claude.lnk")));
    }
}
