//! Minimal client for the Everything 1.4 IPC protocol (EVERYTHING_IPC_QUERY2 over WM_COPYDATA).
//! Talks to the running Everything window directly, so no SDK DLL or es.exe is required.
use crate::{Hit, SearchError};
use std::{
    cell::RefCell,
    ffi::c_void,
    path::PathBuf,
    ptr,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, HWND_MESSAGE, MSG,
        PM_REMOVE, PeekMessageW, RegisterClassW, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
        TranslateMessage, WM_COPYDATA, WNDCLASSW,
    },
};

const EVERYTHING_WINDOW: &str = "EVERYTHING_TASKBAR_NOTIFICATION";
const COPYDATA_QUERY2W: usize = 18;
const REQUEST_NAME: u32 = 0x0000_0001;
const REQUEST_PATH: u32 = 0x0000_0002;
const ITEM_FOLDER: u32 = 0x0000_0001;
const ITEM_DRIVE: u32 = 0x0000_0002;
const SORT_NAME_ASCENDING: u32 = 1;
/// The host waits one second for a search; answer well before that.
const REPLY_TIMEOUT: Duration = Duration::from_millis(700);
const REPLY_BASE: u32 = 0x4D59_0000;

static SEQUENCE: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static WINDOW: RefCell<HWND> = const { RefCell::new(ptr::null_mut()) };
    static REPLY: RefCell<Option<(usize, Vec<u8>)>> = const { RefCell::new(None) };
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if message == WM_COPYDATA && lp != 0 {
        let data = unsafe { &*(lp as *const COPYDATASTRUCT) };
        let bytes = if data.lpData.is_null() || data.cbData == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) }
                .to_vec()
        };
        REPLY.with(|reply| *reply.borrow_mut() = Some((data.dwData, bytes)));
        return 1;
    }
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A hidden message-only window receives Everything's asynchronous reply.
fn reply_window() -> Result<HWND, SearchError> {
    WINDOW.with(|window| {
        let mut window = window.borrow_mut();
        if !window.is_null() {
            return Ok(*window);
        }
        let class = wide("MyToolsEverythingReply");
        let instance = unsafe { GetModuleHandleW(ptr::null()) };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe { RegisterClassW(&wc) };
        let created = unsafe {
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
        if created.is_null() {
            return Err(SearchError::Failed(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        *window = created;
        Ok(created)
    })
}

pub fn search(query: &str, max_results: u32) -> Result<Vec<Hit>, SearchError> {
    let everything = unsafe { FindWindowW(wide(EVERYTHING_WINDOW).as_ptr(), ptr::null()) };
    if everything.is_null() {
        return Err(SearchError::NotRunning);
    }
    let window = reply_window()?;
    let reply_id = REPLY_BASE | (SEQUENCE.fetch_add(1, Ordering::Relaxed) & 0xFFFF);
    let mut request = encode_query(window as usize as u32, reply_id, query, max_results);
    let data = COPYDATASTRUCT {
        dwData: COPYDATA_QUERY2W,
        cbData: request.len() as u32,
        lpData: request.as_mut_ptr() as *mut c_void,
    };
    REPLY.with(|reply| *reply.borrow_mut() = None);
    let mut accepted = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            everything,
            WM_COPYDATA,
            window as WPARAM,
            &data as *const COPYDATASTRUCT as LPARAM,
            SMTO_ABORTIFHUNG,
            REPLY_TIMEOUT.as_millis() as u32,
            &mut accepted,
        )
    };
    if sent == 0 || accepted == 0 {
        return Err(SearchError::Failed("Everything rejected the query".into()));
    }
    let bytes = wait_reply(reply_id as usize)?;
    decode_list(&bytes)
}

fn wait_reply(reply_id: usize) -> Result<Vec<u8>, SearchError> {
    let deadline = Instant::now() + REPLY_TIMEOUT;
    loop {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let reply = REPLY.with(|reply| {
            let mut reply = reply.borrow_mut();
            match reply.take() {
                Some((id, bytes)) if id == reply_id => Some(bytes),
                _ => None, // A late reply to an abandoned query is discarded.
            }
        });
        if let Some(bytes) = reply {
            return Ok(bytes);
        }
        if Instant::now() >= deadline {
            return Err(SearchError::Failed("timeout".into()));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn encode_query(reply_window: u32, reply_id: u32, query: &str, max_results: u32) -> Vec<u8> {
    let header = [
        reply_window,
        reply_id,
        0, // search flags: case-insensitive, no regex
        0, // offset
        max_results,
        REQUEST_NAME | REQUEST_PATH,
        SORT_NAME_ASCENDING,
    ];
    let mut bytes: Vec<u8> = header
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    bytes.extend(wide(query).iter().flat_map(|unit| unit.to_le_bytes()));
    bytes
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SearchError> {
    bytes
        .get(offset..offset + 4)
        .map(|slice| u32::from_le_bytes(slice.try_into().unwrap()))
        .ok_or_else(|| SearchError::Failed("truncated reply".into()))
}

/// Reads a length-prefixed UTF-16 string and returns it with the offset after its terminator.
fn read_string(bytes: &[u8], offset: usize) -> Result<(String, usize), SearchError> {
    let length = read_u32(bytes, offset)? as usize;
    let start = offset + 4;
    let end = start + length * 2;
    let units: Vec<u16> = bytes
        .get(start..end)
        .ok_or_else(|| SearchError::Failed("truncated reply".into()))?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    Ok((String::from_utf16_lossy(&units), end + 2))
}

/// Decodes EVERYTHING_IPC_LIST2: five header DWORDs, then (flags, data_offset) per item.
fn decode_list(bytes: &[u8]) -> Result<Vec<Hit>, SearchError> {
    let count = read_u32(bytes, 4)? as usize;
    let flags = read_u32(bytes, 12)?;
    let mut hits = Vec::with_capacity(count.min(1024));
    for index in 0..count {
        let item = 20 + index * 8;
        let item_flags = read_u32(bytes, item)?;
        let mut offset = read_u32(bytes, item + 4)? as usize;
        let mut name = String::new();
        let mut parent = String::new();
        if flags & REQUEST_NAME != 0 {
            (name, offset) = read_string(bytes, offset)?;
        }
        if flags & REQUEST_PATH != 0 {
            (parent, _) = read_string(bytes, offset)?;
        }
        let drive = item_flags & ITEM_DRIVE != 0;
        let path = if parent.is_empty() {
            // Drives are reported as "C:" without a parent folder.
            PathBuf::from(if drive { format!("{name}\\") } else { name })
        } else {
            PathBuf::from(parent).join(name)
        };
        hits.push(Hit {
            path,
            folder: item_flags & ITEM_FOLDER != 0,
            drive,
        });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut bytes = (units.len() as u32).to_le_bytes().to_vec();
        bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
        bytes.extend([0, 0]);
        bytes
    }

    #[test]
    fn query_header_has_seven_dwords_and_terminated_text() {
        let bytes = encode_query(7, 9, "ab", 50);
        assert_eq!(bytes.len(), 28 + 6);
        assert_eq!(read_u32(&bytes, 0).unwrap(), 7);
        assert_eq!(read_u32(&bytes, 4).unwrap(), 9);
        assert_eq!(read_u32(&bytes, 16).unwrap(), 50);
        assert_eq!(&bytes[28..], &[b'a', 0, b'b', 0, 0, 0]);
    }

    #[test]
    fn decodes_files_folders_and_drives() {
        let mut header: Vec<u8> = [3u32, 3, 0, REQUEST_NAME | REQUEST_PATH, 1]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let entries = [
            (0, "App.sln", r"C:\work"),
            (ITEM_FOLDER, "src", r"C:\work"),
            (ITEM_FOLDER | ITEM_DRIVE, "D:", ""),
        ];
        let mut data = Vec::new();
        let data_start = 20 + entries.len() * 8;
        for (flags, name, path) in entries {
            header.extend(flags.to_le_bytes());
            header.extend(((data_start + data.len()) as u32).to_le_bytes());
            data.extend(string(name));
            data.extend(string(path));
        }
        header.extend(data);
        let hits = decode_list(&header).unwrap();
        assert_eq!(hits[0].path, PathBuf::from(r"C:\work\App.sln"));
        assert!(!hits[0].folder);
        assert!(hits[1].folder);
        assert_eq!(hits[2].path, PathBuf::from(r"D:\"));
        assert!(hits[2].drive);
        assert!(decode_list(&header[..30]).is_err());
    }
}
