//! Opens a PowerShell window that runs a script and stays open, like a terminal the user
//! opened: the user's profile loads and Windows shows it in the default terminal app.
use std::path::Path;

/// Runs `script` in a new PowerShell window starting in `directory`.
pub fn open(script: &str, directory: &Path) -> Result<(), String> {
    #[cfg(windows)]
    return windows::open(&shell(), &parameters(script), directory);
    #[cfg(not(windows))]
    {
        let _ = (script, directory);
        Err("only available on Windows".into())
    }
}

/// `-EncodedCommand` takes Base64 of UTF-16LE, so quotes, `;`, `&` and Chinese text reach
/// PowerShell unchanged instead of being parsed as part of the command line.
#[cfg_attr(not(windows), allow(dead_code))]
fn parameters(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    format!("-NoLogo -NoExit -EncodedCommand {encoded}")
}

/// PowerShell 7 when it is on `PATH`, otherwise the Windows PowerShell of the system.
#[cfg(windows)]
fn shell() -> std::path::PathBuf {
    let on_path = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join("pwsh.exe"))
            .find(|path| path.is_file())
    });
    on_path.unwrap_or_else(|| {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
    })
}

#[cfg(windows)]
mod windows {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};
    use windows_sys::Win32::UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    };

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    /// The shell starts the window, so it gets its own console instead of the plugin's
    /// stdin/stdout, which carry the host protocol.
    pub fn open(executable: &Path, parameters: &str, directory: &Path) -> Result<(), String> {
        let executable = wide(executable.as_os_str());
        let parameters = wide(OsStr::new(parameters));
        let directory = wide(directory.as_os_str());
        let verb = wide(OsStr::new("open"));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
            lpVerb: verb.as_ptr(),
            lpFile: executable.as_ptr(),
            lpParameters: parameters.as_ptr(),
            lpDirectory: directory.as_ptr(),
            nShow: SW_SHOWNORMAL,
            ..Default::default()
        };
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn encodes_the_script_unchanged() {
        let script = "Write-Host '编译 & done'; dotnet build $project";
        let parameters = parameters(script);
        let encoded = parameters
            .strip_prefix("-NoLogo -NoExit -EncodedCommand ")
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let units: Vec<u16> = bytes
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).unwrap(), script);
    }
}
