//! Slint 1.18.1's Windows tray ignores MenuEntry.icon. Decorate its native popup
//! just before display, without replacing Slint's menu actions or event handling.
use crate::{AppTray, i18n};
use std::{cell::RefCell, collections::HashMap, ptr, rc::Rc, time::Duration};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, COLOR_MENUTEXT, CreateDIBSection, DIB_RGB_COLORS,
        DeleteObject, GetSysColor, HBITMAP,
    },
    UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            FindWindowExW, GetMenuItemCount, GetMenuStringW, GetWindowThreadProcessId, HMENU,
            HWND_MESSAGE, MENUITEMINFOW, MF_BYPOSITION, MIIM_BITMAP, MIIM_STRING, SM_CXMENUCHECK,
            SetMenuItemInfoW, WM_INITMENUPOPUP, WM_NCDESTROY,
        },
    },
};

const SUBCLASS_ID: usize = 0x4D545243;

pub struct MenuIcons {
    // Stop polling before releasing the hook and its callback data.
    _timer: Rc<slint::Timer>,
    _hook: Rc<RefCell<Option<Hook>>>,
}

impl MenuIcons {
    pub fn new(tray: &AppTray) -> Self {
        let timer = Rc::new(slint::Timer::default());
        let hook = Rc::new(RefCell::new(None));
        let installed = hook.clone();
        let weak_timer = Rc::downgrade(&timer);
        let weak_tray = tray.as_weak();
        // Slint creates the hidden tray window after entering the event loop.
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let Some(tray) = weak_tray.upgrade() else {
                    return;
                };
                if let Some(hook) = Hook::install(&tray) {
                    *installed.borrow_mut() = Some(hook);
                    tray.set_native_menu_icons(true);
                    if let Some(timer) = weak_timer.upgrade() {
                        timer.stop();
                    }
                }
            },
        );
        Self {
            _timer: timer,
            _hook: hook,
        }
    }
}

struct Hook {
    data: Box<HookData>,
}

struct HookData {
    hwnd: std::cell::Cell<HWND>,
    tray: slint::Weak<AppTray>,
    bitmaps: RefCell<HashMap<String, Bitmap>>,
}

impl Hook {
    fn install(tray: &AppTray) -> Option<Self> {
        let class: Vec<u16> = "SlintSystemTrayWindow\0".encode_utf16().collect();
        let mut hwnd = ptr::null_mut();
        loop {
            // Search only message-only windows, and only attach to this process.
            hwnd = unsafe { FindWindowExW(HWND_MESSAGE, hwnd, class.as_ptr(), ptr::null()) };
            if hwnd.is_null() {
                return None;
            }
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
            if pid == std::process::id() {
                break;
            }
        }
        let data = Box::new(HookData {
            hwnd: std::cell::Cell::new(hwnd),
            tray: tray.as_weak(),
            bitmaps: RefCell::new(HashMap::new()),
        });
        // The Box keeps the callback pointer stable until the subclass is removed.
        let installed = unsafe {
            SetWindowSubclass(
                hwnd,
                Some(menu_proc),
                SUBCLASS_ID,
                &*data as *const HookData as usize,
            )
        };
        (installed != 0).then_some(Self { data })
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        let hwnd = self.data.hwnd.get();
        if !hwnd.is_null() {
            unsafe { RemoveWindowSubclass(hwnd, Some(menu_proc), SUBCLASS_ID) };
        }
    }
}

unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    // Installed and removed on the UI thread; Hook owns the pointed-to allocation.
    let data = unsafe { &*(data as *const HookData) };
    if message == WM_INITMENUPOPUP {
        data.decorate(wparam as HMENU);
    } else if message == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(hwnd, Some(menu_proc), SUBCLASS_ID) };
        data.hwnd.set(ptr::null_mut());
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl HookData {
    fn decorate(&self, menu: HMENU) {
        let Some(tray) = self.tray.upgrade() else {
            return;
        };
        let language = tray.get_language();
        let items = [
            (i18n::t("tray.startup"), tray.get_startup_enabled()),
            (i18n::t("tray.language_auto"), language == "auto"),
            (i18n::t("tray.language_chinese"), language == "zh-CN"),
            (i18n::t("tray.language_english"), language == "en"),
        ];
        for (title, selected) in items {
            if let Ok(bitmap) = decorate_item(
                menu,
                self.hwnd.get(),
                &title,
                selected,
                &tray.get_menu_check_icon(),
            ) {
                // Each submenu owns its own bitmap; opening a submenu must not
                // release the check still referenced by the parent popup.
                let mut bitmaps = self.bitmaps.borrow_mut();
                if let Some(bitmap) = bitmap {
                    bitmaps.insert(title, bitmap);
                } else {
                    bitmaps.remove(&title);
                }
            }
        }
    }
}

fn decorate_item(
    menu: HMENU,
    hwnd: HWND,
    title: &str,
    enabled: bool,
    image: &slint::Image,
) -> Result<Option<Bitmap>, ()> {
    let marked_title = format!("{title}  ✓");
    for position in 0..unsafe { GetMenuItemCount(menu) }.max(0) as u32 {
        let mut text = [0u16; 256];
        let length = unsafe {
            GetMenuStringW(
                menu,
                position,
                text.as_mut_ptr(),
                text.len() as i32,
                MF_BYPOSITION,
            )
        };
        let actual = String::from_utf16_lossy(&text[..length.max(0) as usize]);
        if actual != title && actual != marked_title {
            continue;
        }
        let bitmap = if enabled {
            Bitmap::new(hwnd, image)
        } else {
            None
        };
        // If bitmap creation fails, keep the text check as a visible fallback.
        let label = if enabled && bitmap.is_none() {
            marked_title.as_str()
        } else {
            title
        };
        let mut label: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP | MIIM_STRING,
            hbmpItem: bitmap.as_ref().map_or(ptr::null_mut(), |bitmap| bitmap.0),
            dwTypeData: label.as_mut_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        if unsafe { SetMenuItemInfoW(menu, position, 1, &info) } != 0 {
            return Ok(bitmap);
        }
        return Err(());
    }
    Err(())
}

struct Bitmap(HBITMAP);

impl Bitmap {
    fn new(hwnd: HWND, image: &slint::Image) -> Option<Self> {
        let pixels = image.to_rgba8()?;
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let size =
            unsafe { GetSystemMetricsForDpi(SM_CXMENUCHECK, if dpi == 0 { 96 } else { dpi }) }
                .max(1);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..unsafe { std::mem::zeroed() }
            },
            ..unsafe { std::mem::zeroed() }
        };
        let mut bits = ptr::null_mut();
        let bitmap = unsafe {
            CreateDIBSection(
                ptr::null_mut(),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            )
        };
        if bitmap.is_null() {
            return None;
        }
        let bitmap = Self(bitmap);
        if bits.is_null() || pixels.width() == 0 || pixels.height() == 0 {
            return None;
        }
        let color = unsafe { GetSysColor(COLOR_MENUTEXT) };
        let output =
            unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize) };
        for (index, pixel) in output.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = index % size as usize;
            let y = index / size as usize;
            let sx = x * pixels.width() as usize / size as usize;
            let sy = y * pixels.height() as usize / size as usize;
            let alpha = pixels.as_slice()[sy * pixels.width() as usize + sx].a as u32;
            // Native menu bitmaps use premultiplied BGRA, tinted to system menu text.
            pixel.copy_from_slice(&[
                (((color >> 16) & 255) * alpha / 255) as u8,
                (((color >> 8) & 255) * alpha / 255) as u8,
                ((color & 255) * alpha / 255) as u8,
                alpha as u8,
            ]);
        }
        Some(bitmap)
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemInfoW, MF_STRING,
    };

    struct Popup(HMENU);

    impl Drop for Popup {
        fn drop(&mut self) {
            unsafe { DestroyMenu(self.0) };
        }
    }

    #[test]
    fn startup_menu_uses_lucide_bitmap_and_clears_it_when_disabled() {
        let menu = Popup(unsafe { CreatePopupMenu() });
        assert!(!menu.0.is_null());
        let title: Vec<u16> = "开机启动  ✓\0".encode_utf16().collect();
        assert_ne!(
            unsafe { AppendMenuW(menu.0, MF_STRING, 1, title.as_ptr()) },
            0
        );
        let image = slint::Image::load_from_svg_data(include_bytes!(
            "../../../../ui/assets/icons/check.svg"
        ))
        .unwrap();
        let bitmap = decorate_item(menu.0, ptr::null_mut(), "开机启动", true, &image)
            .unwrap()
            .expect("Lucide check should produce a native bitmap");
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP,
            ..unsafe { std::mem::zeroed() }
        };
        assert_ne!(unsafe { GetMenuItemInfoW(menu.0, 0, 1, &mut info) }, 0);
        assert_eq!(info.hbmpItem, bitmap.0);
        let mut text = [0u16; 64];
        let length = unsafe { GetMenuStringW(menu.0, 0, text.as_mut_ptr(), 64, MF_BYPOSITION) };
        assert_eq!(
            String::from_utf16_lossy(&text[..length as usize]),
            "开机启动"
        );
        assert!(
            decorate_item(menu.0, ptr::null_mut(), "开机启动", false, &image)
                .unwrap()
                .is_none()
        );
        assert_ne!(unsafe { GetMenuItemInfoW(menu.0, 0, 1, &mut info) }, 0);
        assert!(info.hbmpItem.is_null());
        // Keep the bitmap alive until the menu no longer references it.
        drop(bitmap);
    }
}
