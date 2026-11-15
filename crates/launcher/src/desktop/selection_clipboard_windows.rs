//! A temporary snapshot of clipboard formats that can be restored as byte buffers.
use anyhow::{Result, anyhow};
use clipboard_win::{Clipboard, EnumFormats, raw};
use windows::{
    Win32::{
        Foundation::HWND,
        System::DataExchange::{GetClipboardFormatNameW, GetClipboardSequenceNumber},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
        },
    },
    core::{PCWSTR, w},
};

const MAX_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct Snapshot {
    pub(super) sequence: u32,
    formats: Vec<(u32, Vec<u8>)>,
}

impl Snapshot {
    pub(super) fn capture() -> std::result::Result<Self, &'static str> {
        let _clipboard = Clipboard::new().map_err(|_| "clipboard_busy")?;
        let mut formats = Vec::new();
        let mut total = 0usize;
        for format in EnumFormats::new() {
            if !supported_format(format) {
                return Err("clipboard_has_other_formats");
            }
            let size = raw::size(format)
                .ok_or("clipboard_format_unavailable")?
                .get();
            total = total.saturating_add(size);
            if total > MAX_BYTES {
                return Err("clipboard_snapshot_too_large");
            }
            let mut bytes = Vec::with_capacity(size);
            raw::get_vec(format, &mut bytes).map_err(|_| "clipboard_format_unavailable")?;
            formats.push((format, bytes));
        }
        Ok(Self {
            sequence: unsafe { GetClipboardSequenceNumber() },
            formats,
        })
    }

    pub(super) fn restore(self, copied_sequence: u32) -> Result<()> {
        let owner = ClipboardOwner::new()?;
        let _clipboard = Clipboard::new_attempts_for(owner.0.0.cast(), 5)
            .map_err(|_| anyhow!("clipboard_busy_during_restore"))?;
        // Leave a newer clipboard update from the user or another app alone.
        anyhow::ensure!(
            unsafe { GetClipboardSequenceNumber() } == copied_sequence,
            "clipboard_changed_before_restore"
        );
        raw::empty().map_err(|_| anyhow!("clipboard_restore_failed"))?;
        for (format, bytes) in self.formats {
            raw::set_without_clear(format, &bytes)
                .map_err(|_| anyhow!("clipboard_restore_failed"))?;
        }
        Ok(())
    }
}

fn supported_format(format: u32) -> bool {
    // CF_TEXT, CF_OEMTEXT, CF_UNICODETEXT, CF_LOCALE.
    if matches!(format, 1 | 7 | 13 | 16) {
        return true;
    }
    let mut name = [0; 96];
    let len = unsafe { GetClipboardFormatNameW(format, &mut name) }.max(0) as usize;
    matches!(
        String::from_utf16_lossy(&name[..len]).as_str(),
        "HTML Format"
            | "Rich Text Format"
            | "Chromium internal source RFH token"
            | "Chromium internal source URL"
            | "ExcludeClipboardContentFromMonitorProcessing"
            | "CanIncludeInClipboardHistory"
            | "CanUploadToCloudClipboard"
    )
}

struct ClipboardOwner(HWND);

impl ClipboardOwner {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                PCWSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .map(Self)
    }
}

impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}
