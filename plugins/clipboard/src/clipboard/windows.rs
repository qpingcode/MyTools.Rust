use anyhow::{Result, ensure};
#[path = "dib.rs"]
mod dib;
pub use dib::image;
use std::{
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr,
    sync::{
        OnceLock,
        atomic::{AtomicIsize, Ordering},
        mpsc::Sender,
    },
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::{
            AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
            IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        LibraryLoader::GetModuleHandleW,
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    },
    UI::{
        Shell::{DROPFILES, DragQueryFileW},
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE,
            RegisterClassW, TranslateMessage, WM_CLIPBOARDUPDATE, WNDCLASSW,
        },
    },
};

static CHANGES: OnceLock<Sender<()>> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);
const FILE_DROP: u32 = 15;

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        if let Some(sender) = CHANGES.get() {
            let _ = sender.send(());
        }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}

pub fn watch(sender: Sender<()>) -> Result<()> {
    let _ = CHANGES.set(sender);
    let class: Vec<u16> = "MyToolsClipboardListener\0".encode_utf16().collect();
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        RegisterClassW(&wc);
    }
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    ensure!(!window.is_null(), std::io::Error::last_os_error());
    WINDOW.store(window as isize, Ordering::Relaxed);
    ensure!(
        unsafe { AddClipboardFormatListener(window) } != 0,
        std::io::Error::last_os_error()
    );
    // Include content copied before the listener was started.
    if let Some(sender) = CHANGES.get() {
        let _ = sender.send(());
    }
    let mut message = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

struct Open;
impl Open {
    fn new() -> Result<Self> {
        ensure!(
            unsafe { OpenClipboard(WINDOW.load(Ordering::Relaxed) as HWND) } != 0,
            std::io::Error::last_os_error()
        );
        Ok(Self)
    }
}
impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

pub fn excluded() -> bool {
    let name: Vec<u16> = "ExcludeClipboardContentFromMonitorProcessing\0"
        .encode_utf16()
        .collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if unsafe { IsClipboardFormatAvailable(format) } != 0 {
        return true;
    }
    let name: Vec<u16> = "CanIncludeInClipboardHistory\0".encode_utf16().collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if unsafe { IsClipboardFormatAvailable(format) } == 0 {
        return false;
    }
    let Ok(_open) = Open::new() else {
        return true;
    };
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return true;
    }
    if unsafe { GlobalSize(handle) } < size_of::<u32>() {
        return true;
    }
    let value = unsafe { GlobalLock(handle) };
    if value.is_null() {
        return true;
    }
    let excluded = unsafe { (value as *const u32).read_unaligned() } == 0;
    unsafe {
        GlobalUnlock(handle);
    }
    excluded
}

pub fn files() -> Result<Option<Vec<PathBuf>>> {
    if unsafe { IsClipboardFormatAvailable(FILE_DROP) } == 0 {
        return Ok(None);
    }
    let _open = Open::new()?;
    let handle = unsafe { GetClipboardData(FILE_DROP) };
    ensure!(!handle.is_null(), std::io::Error::last_os_error());
    let count = unsafe { DragQueryFileW(handle as _, u32::MAX, ptr::null_mut(), 0) };
    ensure!(count <= 1024, crate::i18n::text("too_large"));
    let mut files = Vec::new();
    for index in 0..count.min(1024) {
        let size = unsafe { DragQueryFileW(handle as _, index, ptr::null_mut(), 0) };
        let mut path = vec![0; size as usize + 1];
        unsafe {
            DragQueryFileW(handle as _, index, path.as_mut_ptr(), size + 1);
        }
        files.push(PathBuf::from(std::ffi::OsString::from_wide(
            &path[..size as usize],
        )));
    }
    Ok(Some(files))
}

pub fn sequence() -> u64 {
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() as u64 }
}

pub fn restore_files(paths: &[PathBuf]) -> Result<()> {
    let names: Vec<u16> = paths
        .iter()
        .flat_map(|path| path.as_os_str().encode_wide().chain(Some(0)))
        .chain(Some(0))
        .collect();
    let size = size_of::<DROPFILES>() + names.len() * 2;
    let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, size) };
    ensure!(!handle.is_null(), std::io::Error::last_os_error());
    let result = (|| -> Result<()> {
        let bytes = unsafe { GlobalLock(handle) };
        ensure!(!bytes.is_null(), std::io::Error::last_os_error());
        unsafe {
            (bytes as *mut DROPFILES).write(DROPFILES {
                pFiles: size_of::<DROPFILES>() as u32,
                fWide: 1,
                ..std::mem::zeroed()
            });
            ptr::copy_nonoverlapping(
                names.as_ptr() as *const u8,
                (bytes as *mut u8).add(size_of::<DROPFILES>()),
                names.len() * 2,
            );
            GlobalUnlock(handle);
        }
        let _open = Open::new()?;
        ensure!(
            unsafe { EmptyClipboard() } != 0,
            std::io::Error::last_os_error()
        );
        ensure!(
            !unsafe { SetClipboardData(FILE_DROP, handle) }.is_null(),
            std::io::Error::last_os_error()
        );
        Ok(())
    })();
    if result.is_err() {
        unsafe {
            GlobalFree(handle);
        }
    }
    result
}
