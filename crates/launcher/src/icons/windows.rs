use super::Pixels;
use std::{os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
    },
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    UI::{
        Controls::{ILD_TRANSPARENT, ImageList_GetIcon},
        Shell::{SHFILEINFOW, SHGFI_LARGEICON, SHGFI_SYSICONINDEX, SHGetFileInfoW},
        WindowsAndMessaging::{DI_MASK, DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
    },
};

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0) };
    }
}

struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn draw(icon: HICON, flags: u32) -> Option<Vec<u8>> {
    const SIZE: usize = 48;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE as i32,
            biHeight: -(SIZE as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { std::mem::zeroed() }
        },
        ..unsafe { std::mem::zeroed() }
    };
    let mut bits = ptr::null_mut();
    let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let bitmap =
        unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0) };
    if bitmap.is_null() {
        unsafe {
            DeleteDC(dc);
        }
        return None;
    }
    let old = unsafe { SelectObject(dc, bitmap) };
    let _dib = Dib { dc, bitmap, old };
    if bits.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, SIZE * SIZE * 4) };
    bytes.fill(0);
    if unsafe {
        DrawIconEx(
            dc,
            0,
            0,
            icon,
            SIZE as i32,
            SIZE as i32,
            0,
            ptr::null_mut(),
            flags,
        )
    } == 0
    {
        return None;
    }
    // Finish any batched GDI writes before reading the DIB's backing memory.
    unsafe {
        GdiFlush();
    }
    Some(bytes.to_vec())
}

pub fn load(path: &Path) -> Option<Pixels> {
    if unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) } < 0 {
        return None;
    }
    let _apartment = Apartment;
    // SHGetFileInfo is a shell API, so remove filesystem-only verbatim prefixes.
    let path = path.to_string_lossy().replace('/', "\\");
    let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    let path: Vec<u16> = std::ffi::OsStr::new(&path)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    // SHGFI_ICON always adds an arrow for shortcuts. Request only the system
    // image-list index, then copy its base icon without any overlay flags.
    // This preserves custom shortcut icons instead of guessing the target icon.
    let images = unsafe {
        SHGetFileInfoW(
            path.as_ptr(),
            0,
            &mut info,
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_SYSICONINDEX | SHGFI_LARGEICON,
        )
    };
    if images == 0 {
        return None;
    }
    // The image list belongs to Shell; only the copied HICON is ours to destroy.
    let handle = unsafe { ImageList_GetIcon(images as _, info.iIcon, ILD_TRANSPARENT) };
    if handle.is_null() {
        return None;
    }
    let icon = Icon(handle);
    let mut rgba = draw(icon.0, DI_NORMAL)?;
    let legacy = rgba.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0);
    let mask = if legacy {
        Some(draw(icon.0, DI_MASK)?)
    } else {
        None
    };
    for (index, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        pixel.swap(0, 2);
        if let Some(mask) = &mask {
            pixel[3] = if mask[index * 4] == 0 { 255 } else { 0 };
        } else if pixel[3] > 0 {
            let alpha = pixel[3] as u32;
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 / alpha).min(255)) as u8;
            }
        }
    }
    Some(Pixels {
        width: 48,
        height: 48,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_keeps_custom_icon_without_link_overlay() {
        use windows::{
            Win32::{
                System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile},
                UI::Shell::{IShellLinkW, ShellLink},
            },
            core::{Interface, PCWSTR},
        };
        assert!(unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0);
        let _apartment = Apartment;
        let root =
            std::env::temp_dir().join(format!("mytools-overlay-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let system = Path::new(&std::env::var_os("WINDIR").unwrap()).join("System32");
        let custom_icon = system.join("notepad.exe");
        let target = system.join("cmd.exe");
        let shortcut = root.join("测试 shortcut.lnk");
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<u16>>()
        };
        let link: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap() };
        let file: IPersistFile = link.cast().unwrap();
        unsafe {
            link.SetPath(PCWSTR(wide(&target).as_ptr())).unwrap();
            link.SetIconLocation(PCWSTR(wide(&custom_icon).as_ptr()), 0)
                .unwrap();
            file.Save(PCWSTR(wide(&shortcut).as_ptr()), true).unwrap();
        }
        let expected = load(&custom_icon).unwrap();
        let actual = load(&shortcut).unwrap();
        assert_eq!(
            actual.rgba, expected.rgba,
            "custom shortcut icon must match its source with no overlay"
        );
        drop(file);
        drop(link);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extracts_native_icon_from_windows_executable() {
        let path = Path::new(&std::env::var_os("WINDIR").unwrap()).join("System32/notepad.exe");
        let image = load(&path).expect("Windows shell should provide an executable icon");
        assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
        assert!(
            image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] > 0)
        );
    }
}
