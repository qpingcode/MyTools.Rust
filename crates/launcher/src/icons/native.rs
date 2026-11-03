use std::path::Path;

pub struct Pixels {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) RGBA8.
    pub rgba: Vec<u8>,
}

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

pub fn load(path: &Path) -> Option<Pixels> {
    if !path.is_absolute() || !path.exists() {
        return None;
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        platform::load(path)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}
