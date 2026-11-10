//! Crash reports for MyTools and its plugins. The launcher runs without a console and
//! plugins have no stderr, so panics, native exceptions and fatal errors are appended to
//! `logs/crash.log` next to `mytools.exe`, where they survive the process.
use std::{
    fmt::Display,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

/// The host passes its log directory to every plugin process in this variable.
pub const ENV: &str = "MYTOOLS_LOG_DIR";
const FILE: &str = "crash.log";
/// A larger log is moved to `crash.log.1` before the next report, keeping at most two.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// The host's log directory: inherited from the host, else `logs` next to this executable.
pub fn directory() -> PathBuf {
    std::env::var_os(ENV).map(PathBuf::from).unwrap_or_else(|| {
        std::env::current_exe()
            .unwrap_or_default()
            .with_file_name("logs")
    })
}

/// Records panics (and on Windows, unhandled native exceptions) before the process dies.
/// The previous panic hook still runs, so stderr output and test reports are unchanged.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let report = format!(
            "panic on thread '{}' at {}:\n{}\n\nbacktrace:\n{}",
            thread.name().unwrap_or("<unnamed>"),
            info.location()
                .map_or_else(|| "<unknown>".to_owned(), ToString::to_string),
            panic_message(info.payload()),
            std::backtrace::Backtrace::force_capture(),
        );
        write_report(&report);
        previous(info);
    }));
    #[cfg(windows)]
    native::install();
}

/// Records an error that ends the process, such as one returned from `main`.
pub fn report_error(error: &dyn Display) {
    write_report(&format!("fatal error:\n{error}"));
}

fn panic_message(payload: &dyn std::any::Any) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>")
}

/// Best effort: a crash report must never cause a second failure.
fn write_report(body: &str) {
    let _ = append(&directory(), &format!("{}\n{body}\n\n", header()));
}

fn header() -> String {
    let process = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    format!(
        "==== {} {process} {} (pid {}) ====",
        timestamp(),
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    )
}

fn append(directory: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(FILE);
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, directory.join(format!("{FILE}.1")));
    }
    // One write per report keeps reports from concurrent processes whole.
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

#[cfg(windows)]
fn timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime only writes the SYSTEMTIME it is given.
    let time = unsafe {
        let mut time = std::mem::zeroed();
        GetLocalTime(&mut time);
        time
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

#[cfg(not(windows))]
fn timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("unix {seconds}")
}

#[cfg(windows)]
mod native {
    use windows_sys::Win32::{
        Foundation::HMODULE,
        System::{
            Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter},
            LibraryLoader::{
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
                GetModuleHandleExW,
            },
        },
    };

    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    pub fn install() {
        // SAFETY: the filter is a plain function valid for the whole process lifetime.
        unsafe { SetUnhandledExceptionFilter(Some(filter)) };
    }

    /// Access violations and other native faults that bypass Rust's panic machinery.
    /// Windows still terminates the process (and reports it) after this returns.
    unsafe extern "system" fn filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
        // SAFETY: Windows passes valid exception pointers to the top-level filter.
        if let Some(record) = unsafe { pointers.as_ref().and_then(|p| p.ExceptionRecord.as_ref()) }
        {
            let address = record.ExceptionAddress as usize;
            super::write_report(&format!(
                "native exception 0x{:08X} ({}) at 0x{address:X} {}",
                record.ExceptionCode as u32,
                describe(record.ExceptionCode as u32),
                module_offset(address),
            ));
        }
        EXCEPTION_CONTINUE_SEARCH
    }

    fn describe(code: u32) -> &'static str {
        match code {
            0xC000_0005 => "access violation",
            0xC000_001D => "illegal instruction",
            0xC000_0094 => "integer divide by zero",
            0xC000_00FD => "stack overflow",
            0xC000_0374 => "heap corruption",
            0xC000_0409 => "stack buffer overrun / fast fail",
            0x8000_0003 => "breakpoint",
            _ => "unknown",
        }
    }

    /// `(module+0xoffset)` locates the fault in a binary regardless of where it was loaded.
    fn module_offset(address: usize) -> String {
        let mut module: HMODULE = std::ptr::null_mut();
        let flags =
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        // SAFETY: with FROM_ADDRESS the name argument is an address; module is an out value.
        if unsafe { GetModuleHandleExW(flags, address as *const u16, &mut module) } == 0 {
            return String::new();
        }
        let mut name = [0u16; 260];
        // SAFETY: the buffer length matches the slice.
        let len = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) };
        let name = String::from_utf16_lossy(&name[..len as usize]);
        format!("({name}+0x{:X})", address - module as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_string_and_str_panic_messages() {
        assert_eq!(panic_message(&"boom"), "boom");
        assert_eq!(panic_message(&String::from("bang")), "bang");
        assert_eq!(panic_message(&42), "<non-string panic payload>");
    }

    #[test]
    fn appends_reports_and_rotates_large_logs() {
        let directory = std::env::temp_dir().join(format!("mytools-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        append(&directory, "first\n").unwrap();
        append(&directory, "second\n").unwrap();
        let log = directory.join(FILE);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "first\nsecond\n");

        std::fs::write(&log, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        append(&directory, "third\n").unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "third\n");
        assert!(directory.join(format!("{FILE}.1")).exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
