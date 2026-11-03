use std::path::Path;

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
