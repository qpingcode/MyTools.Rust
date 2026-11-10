//! Slint 1.18.1's Windows tray draws a plain native popup: it ignores MenuEntry.icon, and
//! its text uses the system menu font, whose fallback for Chinese is often a Japanese font
//! (Meiryo UI) on non-Chinese Windows. Just before display, its items become owner-drawn,
//! painted in Microsoft YaHei UI with a Lucide check, without replacing Slint's menu
//! actions or event handling. Separators and submenu arrows stay native.
use crate::{AppTray, i18n};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ptr,
    rc::Rc,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
        CLR_INVALID, COLOR_GRAYTEXT, COLOR_MENU, COLOR_MENUTEXT, CreateCompatibleDC,
        CreateDIBSection, CreateFontIndirectW, CreateSolidBrush, DIB_RGB_COLORS, DT_LEFT,
        DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FillRect, GetDC,
        GetPixel, GetStockObject, GetSysColor, GetTextExtentPoint32W, HBITMAP, HDC, HFONT,
        NULL_PEN, ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{
            DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED, ODT_MENU,
        },
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            FindWindowExW, GetMenuItemCount, GetMenuItemInfoW, GetMenuStringW,
            GetWindowThreadProcessId, HMENU, HWND_MESSAGE, MENUITEMINFOW, MF_BYPOSITION,
            MFT_OWNERDRAW, MFT_SEPARATOR, MIIM_DATA, MIIM_FTYPE, NONCLIENTMETRICSW, SM_CXMENUCHECK,
            SPI_GETNONCLIENTMETRICS, SetMenuItemInfoW, SystemParametersInfoW, WM_DRAWITEM,
            WM_INITMENUPOPUP, WM_MEASUREITEM, WM_NCDESTROY,
        },
    },
};

/// The face used for menu text; it covers Chinese with Chinese glyphs and Latin text too.
const MENU_FACE: &str = "Microsoft YaHei UI";
/// Spacing in pixels at 96 DPI.
const EDGE: i32 = 8;
const GAP: i32 = 8;
/// Room on the right for the native submenu arrow.
const TRAILING: i32 = 28;
const VERTICAL_PADDING: i32 = 5;

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
    hwnd: Cell<HWND>,
    tray: slint::Weak<AppTray>,
    /// Owner-drawn items by the id stored in their item data. Slint keeps a popup until it
    /// rebuilds the menu, so items stay owner-drawn across openings.
    labels: RefCell<HashMap<usize, Label>>,
    next_id: Cell<usize>,
    font: RefCell<Option<Font>>,
    check: RefCell<Option<Bitmap>>,
    /// The popup's background as the system painted it, used to clear a highlight.
    background: Cell<Option<COLORREF>>,
}

struct Label {
    title: String,
    text: Vec<u16>,
    checked: bool,
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
        let data = Box::new(HookData::new(hwnd, tray.as_weak()));
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
    match message {
        WM_INITMENUPOPUP => data.decorate(wparam as HMENU),
        WM_MEASUREITEM => {
            let item = unsafe { &mut *(lparam as *mut MEASUREITEMSTRUCT) };
            if item.CtlType == ODT_MENU && data.measure(item) {
                return 1;
            }
        }
        WM_DRAWITEM => {
            let item = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if item.CtlType == ODT_MENU && data.draw(item) {
                return 1;
            }
        }
        WM_NCDESTROY => {
            unsafe { RemoveWindowSubclass(hwnd, Some(menu_proc), SUBCLASS_ID) };
            data.hwnd.set(ptr::null_mut());
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

impl HookData {
    fn new(hwnd: HWND, tray: slint::Weak<AppTray>) -> Self {
        Self {
            hwnd: Cell::new(hwnd),
            tray,
            labels: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            font: RefCell::new(None),
            check: RefCell::new(None),
            background: Cell::new(None),
        }
    }

    /// Makes every item of a popup about to open owner-drawn and updates its check mark.
    fn decorate(&self, menu: HMENU) {
        let Some(tray) = self.tray.upgrade() else {
            return;
        };
        let language = tray.get_language();
        let checked = [
            (i18n::t("tray.startup"), tray.get_startup_enabled()),
            (i18n::t("tray.language_auto"), language == "auto"),
            (i18n::t("tray.language_chinese"), language == "zh-CN"),
            (i18n::t("tray.language_english"), language == "en"),
        ];
        if self.check.borrow().is_none() {
            *self.check.borrow_mut() = Bitmap::new(self.hwnd.get(), &tray.get_menu_check_icon());
        }
        // Sample the background afresh for each popup; the theme may have changed.
        self.background.set(None);
        self.own_items(menu, |title| {
            checked.iter().any(|(name, on)| *on && name == title)
        });
    }

    fn own_items(&self, menu: HMENU, checked: impl Fn(&str) -> bool) {
        for position in 0..unsafe { GetMenuItemCount(menu) }.max(0) as u32 {
            let mut info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_DATA,
                ..unsafe { std::mem::zeroed() }
            };
            if unsafe { GetMenuItemInfoW(menu, position, 1, &mut info) } == 0
                || info.fType & MFT_SEPARATOR != 0
            {
                continue;
            }
            let known = info.fType & MFT_OWNERDRAW != 0
                && self.labels.borrow().contains_key(&info.dwItemData);
            let id = if known {
                info.dwItemData
            } else {
                let Some(title) = item_text(menu, position) else {
                    continue;
                };
                let id = self.next_id.get();
                let owned = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE | MIIM_DATA,
                    fType: info.fType | MFT_OWNERDRAW,
                    dwItemData: id,
                    ..unsafe { std::mem::zeroed() }
                };
                if unsafe { SetMenuItemInfoW(menu, position, 1, &owned) } == 0 {
                    continue;
                }
                self.next_id.set(id + 1);
                self.labels.borrow_mut().insert(
                    id,
                    Label {
                        text: title.encode_utf16().collect(),
                        title,
                        checked: false,
                    },
                );
                id
            };
            if let Some(label) = self.labels.borrow_mut().get_mut(&id) {
                label.checked = checked(&label.title);
            }
        }
    }

    fn scale(&self, value: i32) -> i32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd.get()) };
        let dpi = if dpi == 0 { 96 } else { dpi as i32 };
        value * dpi / 96
    }

    fn check_size(&self) -> i32 {
        let dpi = unsafe { GetDpiForWindow(self.hwnd.get()) };
        unsafe { GetSystemMetricsForDpi(SM_CXMENUCHECK, if dpi == 0 { 96 } else { dpi }) }.max(1)
    }

    /// Where the text starts: the edge, the check column and a gap.
    fn text_left(&self) -> i32 {
        self.scale(EDGE) + self.check_size() + self.scale(GAP)
    }

    fn font(&self) -> HFONT {
        if self.font.borrow().is_none() {
            *self.font.borrow_mut() = Font::menu();
        }
        self.font
            .borrow()
            .as_ref()
            .map_or(ptr::null_mut(), |font| font.0)
    }

    fn measure(&self, item: &mut MEASUREITEMSTRUCT) -> bool {
        let labels = self.labels.borrow();
        let Some(label) = labels.get(&item.itemData) else {
            return false;
        };
        let dc = unsafe { GetDC(ptr::null_mut()) };
        if dc.is_null() {
            return false;
        }
        let old = unsafe { SelectObject(dc, self.font()) };
        let mut size = SIZE { cx: 0, cy: 0 };
        let measured = unsafe {
            GetTextExtentPoint32W(dc, label.text.as_ptr(), label.text.len() as i32, &mut size)
        } != 0;
        unsafe {
            SelectObject(dc, old);
            ReleaseDC(ptr::null_mut(), dc);
        }
        if !measured {
            return false;
        }
        item.itemWidth = (self.text_left() + size.cx + self.scale(TRAILING)) as u32;
        item.itemHeight = (size.cy + 2 * self.scale(VERTICAL_PADDING))
            .max(self.check_size() + 2 * self.scale(VERTICAL_PADDING))
            as u32;
        true
    }

    fn draw(&self, item: &DRAWITEMSTRUCT) -> bool {
        let labels = self.labels.borrow();
        let Some(label) = labels.get(&item.itemData) else {
            return false;
        };
        let dc = item.hDC;
        let rect = item.rcItem;
        let selected = item.itemState & ODS_SELECTED != 0;
        let disabled = item.itemState & (ODS_GRAYED | ODS_DISABLED) != 0;
        let background = self.background_of(dc, &rect);
        let text_color = unsafe {
            GetSysColor(if disabled {
                COLOR_GRAYTEXT
            } else {
                COLOR_MENUTEXT
            })
        };
        fill(dc, &rect, background);
        if selected && !disabled {
            // A soft rounded highlight like the system's modern menus.
            let inset = self.scale(4);
            let radius = self.scale(8);
            let brush = unsafe { CreateSolidBrush(mix(background, text_color, 26)) };
            unsafe {
                let old_brush = SelectObject(dc, brush);
                let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
                RoundRect(
                    dc,
                    rect.left + inset,
                    rect.top + 1,
                    rect.right - inset + 1,
                    rect.bottom,
                    radius,
                    radius,
                );
                SelectObject(dc, old_pen);
                SelectObject(dc, old_brush);
                DeleteObject(brush);
            }
        }
        if label.checked
            && let Some(check) = self.check.borrow().as_ref()
        {
            let top = rect.top + (rect.bottom - rect.top - check.size) / 2;
            check.draw(dc, rect.left + self.scale(EDGE), top);
        }
        let mut text_rect = RECT {
            left: rect.left + self.text_left(),
            ..rect
        };
        unsafe {
            let old = SelectObject(dc, self.font());
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, text_color);
            DrawTextW(
                dc,
                label.text.as_ptr(),
                label.text.len() as i32,
                &mut text_rect,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
            SelectObject(dc, old);
        }
        true
    }

    /// The popup background: read from a corner the highlight never covers before the
    /// first item is painted over. A pixel that cannot be read, or that is as dark as the
    /// text (an unpainted buffer), gives way to the system menu color.
    fn background_of(&self, dc: HDC, rect: &RECT) -> COLORREF {
        if let Some(color) = self.background.get() {
            return color;
        }
        let sampled = unsafe { GetPixel(dc, rect.right - 1, rect.top) };
        let text = unsafe { GetSysColor(COLOR_MENUTEXT) };
        let color = if sampled == CLR_INVALID || distance(sampled, text) < 96 {
            unsafe { GetSysColor(COLOR_MENU) }
        } else {
            sampled
        };
        self.background.set(Some(color));
        color
    }
}

/// The item's text, without the textual check Slint adds when native icons are off.
fn item_text(menu: HMENU, position: u32) -> Option<String> {
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
    if length <= 0 {
        return None;
    }
    let text = String::from_utf16_lossy(&text[..length as usize]);
    Some(text.strip_suffix("  ✓").unwrap_or(&text).to_owned())
}

fn fill(dc: HDC, rect: &RECT, color: COLORREF) {
    let brush = unsafe { CreateSolidBrush(color) };
    unsafe {
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

/// `base` moved towards `toward` by `amount`/255, per channel.
fn mix(base: COLORREF, toward: COLORREF, amount: u32) -> COLORREF {
    let channel = |shift: u32| {
        let (a, b) = ((base >> shift) & 255, (toward >> shift) & 255);
        (a * (255 - amount) + b * amount) / 255
    };
    channel(0) | channel(8) << 8 | channel(16) << 16
}

/// The sum of per-channel differences, 0 to 765.
fn distance(a: COLORREF, b: COLORREF) -> u32 {
    [0, 8, 16]
        .into_iter()
        .map(|shift| ((a >> shift) & 255).abs_diff((b >> shift) & 255))
        .sum()
}

/// The system menu font at its size and weight, in `MENU_FACE`.
struct Font(HFONT);

impl Font {
    fn menu() -> Option<Self> {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                (&raw mut metrics).cast(),
                0,
            )
        };
        if read == 0 {
            return None;
        }
        let mut font = metrics.lfMenuFont;
        font.lfFaceName = [0; 32];
        for (slot, unit) in font.lfFaceName.iter_mut().zip(MENU_FACE.encode_utf16()) {
            *slot = unit;
        }
        let handle = unsafe { CreateFontIndirectW(&font) };
        (!handle.is_null()).then_some(Self(handle))
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0) };
    }
}

/// A Lucide icon as a premultiplied, alpha-blended menu bitmap.
struct Bitmap {
    handle: HBITMAP,
    size: i32,
}

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
        let handle = unsafe {
            CreateDIBSection(
                ptr::null_mut(),
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                ptr::null_mut(),
                0,
            )
        };
        if handle.is_null() {
            return None;
        }
        let bitmap = Self { handle, size };
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
            // Premultiplied BGRA, tinted to system menu text.
            pixel.copy_from_slice(&[
                (((color >> 16) & 255) * alpha / 255) as u8,
                (((color >> 8) & 255) * alpha / 255) as u8,
                ((color & 255) * alpha / 255) as u8,
                alpha as u8,
            ]);
        }
        Some(bitmap)
    }

    fn draw(&self, dc: HDC, x: i32, y: i32) {
        unsafe {
            let source = CreateCompatibleDC(dc);
            if source.is_null() {
                return;
            }
            let old = SelectObject(source, self.handle);
            AlphaBlend(
                dc,
                x,
                y,
                self.size,
                self.size,
                source,
                0,
                0,
                self.size,
                self.size,
                BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                },
            );
            SelectObject(source, old);
            DeleteDC(source);
        }
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, MF_SEPARATOR, MF_STRING,
    };

    struct Popup(HMENU);

    impl Drop for Popup {
        fn drop(&mut self) {
            unsafe { DestroyMenu(self.0) };
        }
    }

    fn item_type(menu: HMENU, position: u32) -> MENUITEMINFOW {
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_DATA,
            ..unsafe { std::mem::zeroed() }
        };
        assert_ne!(unsafe { GetMenuItemInfoW(menu, position, 1, &mut info) }, 0);
        info
    }

    #[test]
    fn items_become_owner_drawn_once_and_follow_their_check_state() {
        let menu = Popup(unsafe { CreatePopupMenu() });
        assert!(!menu.0.is_null());
        let title: Vec<u16> = "开机启动  ✓\0".encode_utf16().collect();
        assert_ne!(
            unsafe { AppendMenuW(menu.0, MF_STRING, 1, title.as_ptr()) },
            0
        );
        assert_ne!(
            unsafe { AppendMenuW(menu.0, MF_SEPARATOR, 0, ptr::null()) },
            0
        );
        let data = HookData::new(ptr::null_mut(), slint::Weak::default());
        data.own_items(menu.0, |title| title == "开机启动");
        let info = item_type(menu.0, 0);
        assert_ne!(info.fType & MFT_OWNERDRAW, 0);
        assert_eq!(
            item_type(menu.0, 1).fType & MFT_OWNERDRAW,
            0,
            "separators stay native"
        );
        {
            let labels = data.labels.borrow();
            let label = &labels[&info.dwItemData];
            assert_eq!(label.title, "开机启动");
            assert!(label.checked);
        }
        // Opening again reuses the label instead of reading text from an owner-drawn item.
        data.own_items(menu.0, |_| false);
        assert_eq!(item_type(menu.0, 0).dwItemData, info.dwItemData);
        assert_eq!(data.labels.borrow().len(), 1);
        assert!(!data.labels.borrow()[&info.dwItemData].checked);
    }

    #[test]
    fn highlight_mixes_towards_the_text_color() {
        assert_eq!(mix(0x00ff_ffff, 0, 0), 0x00ff_ffff);
        assert_eq!(mix(0x00ff_ffff, 0, 255), 0);
        assert_eq!(mix(0x0000_00ff, 0x00ff_0000, 255), 0x00ff_0000);
        assert_eq!(distance(0x00ff_ffff, 0), 765);
        assert_eq!(distance(0x0010_2030, 0x0010_2030), 0);
    }
}
