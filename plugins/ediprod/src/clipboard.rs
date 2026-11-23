//! Reads and writes the Windows clipboard formats a hyperlink is pasted from.
use std::{ptr, thread, time::Duration};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HANDLE},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
            OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Ole::CF_UNICODETEXT,
    },
};

/// What the clipboard holds now: its `HTML Format` document and its plain text.
pub struct Content {
    pub html: Option<String>,
    pub text: Option<String>,
}

pub fn read() -> Result<Content, String> {
    let _open = Open::new()?;
    let html = data(format("HTML Format")).map(|bytes| {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    });
    let text = data(CF_UNICODETEXT as u32).map(|bytes| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|&unit| unit != 0)
            .collect();
        String::from_utf16_lossy(&units)
    });
    Ok(Content { html, text })
}

/// Replaces the clipboard with `text`, an `HTML Format` document and RTF, all at once.
pub fn write(text: &str, cf_html: &str, rtf: &str) -> Result<(), String> {
    let unicode: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let items = [
        (CF_UNICODETEXT as u32, unicode),
        (format("HTML Format"), nul_terminated(cf_html)),
        (format("Rich Text Format"), nul_terminated(rtf)),
    ];
    let _open = Open::new()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(last_error());
    }
    for (format, bytes) in items {
        let handle = global(&bytes)?;
        // On success the clipboard owns the memory; otherwise it is still ours to free.
        if unsafe { SetClipboardData(format, handle) }.is_null() {
            let error = last_error();
            unsafe { GlobalFree(handle) };
            return Err(error);
        }
    }
    Ok(())
}

fn nul_terminated(text: &str) -> Vec<u8> {
    text.bytes().chain([0]).collect()
}

fn format(name: &str) -> u32 {
    let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

/// A copy of the clipboard's data in `format`, while the clipboard is open.
fn data(format: u32) -> Option<Vec<u8>> {
    if unsafe { IsClipboardFormatAvailable(format) } == 0 {
        return None;
    }
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    let size = unsafe { GlobalSize(handle) };
    let bytes = unsafe { GlobalLock(handle) } as *const u8;
    if bytes.is_null() {
        return None;
    }
    let copy = unsafe { std::slice::from_raw_parts(bytes, size) }.to_vec();
    unsafe { GlobalUnlock(handle) };
    Some(copy)
}

fn global(bytes: &[u8]) -> Result<HANDLE, String> {
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if handle.is_null() {
        return Err(last_error());
    }
    let target = unsafe { GlobalLock(handle) } as *mut u8;
    if target.is_null() {
        let error = last_error();
        unsafe { GlobalFree(handle) };
        return Err(error);
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        GlobalUnlock(handle);
    }
    Ok(handle)
}

fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}

/// The clipboard, open until dropped. Another application may hold it for a moment (the
/// clipboard history reads every change), so opening is retried briefly.
struct Open;

impl Open {
    fn new() -> Result<Self, String> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
                return Ok(Self);
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err(last_error())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}
