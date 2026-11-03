//! Per-user login startup. Nothing is registered until the user enables it.
use anyhow::Result;

#[cfg(windows)]
mod platform {
    use anyhow::{Result, ensure};
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
            RegSetKeyValueW,
        },
    };

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn key() -> Vec<u16> {
        wide(OsStr::new(r"Software\Microsoft\Windows\CurrentVersion\Run"))
    }
    fn name() -> Vec<u16> {
        wide(OsStr::new("MyTools.Rust"))
    }
    fn command() -> Result<Vec<u16>> {
        let exe = std::env::current_exe()?;
        let mut text = vec![u16::from(b'"')];
        text.extend(exe.as_os_str().encode_wide());
        text.extend([u16::from(b'"'), 0]);
        ensure!(
            text.len() <= 261,
            "Startup command exceeds the Windows Run key limit"
        );
        Ok(text)
    }

    fn registered() -> Result<Option<Vec<u16>>> {
        let (key, name) = (key(), name());
        let mut size = 0;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut size,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        ensure!(
            result == ERROR_SUCCESS,
            "{}",
            std::io::Error::from_raw_os_error(result as i32)
        );
        let mut data = vec![0u16; (size as usize).div_ceil(2)];
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        ensure!(
            result == ERROR_SUCCESS,
            "{}",
            std::io::Error::from_raw_os_error(result as i32)
        );
        data.truncate((size as usize).div_ceil(2));
        Ok(Some(data))
    }

    pub fn enabled() -> Result<bool> {
        Ok(registered()?.as_ref() == Some(&command()?))
    }

    fn write(command: Option<&[u16]>) -> Result<()> {
        let (key, name) = (key(), name());
        let result = if let Some(command) = command {
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        } else {
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) }
        };
        ensure!(
            result == ERROR_SUCCESS || (command.is_none() && result == ERROR_FILE_NOT_FOUND),
            "{}",
            std::io::Error::from_raw_os_error(result as i32)
        );
        Ok(())
    }

    pub fn set(enabled: bool) -> Result<()> {
        write(if enabled { Some(command()?) } else { None }.as_deref())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "temporarily changes the current user's MyTools startup value and restores it"]
        fn windows_startup_registration_round_trip() {
            struct Restore(Option<Vec<u16>>);
            impl Drop for Restore {
                fn drop(&mut self) {
                    write(self.0.as_deref()).expect("restore startup registration");
                }
            }
            let _restore = Restore(registered().unwrap());
            set(true).unwrap();
            assert!(enabled().unwrap());
            let registered = registered().unwrap().unwrap();
            assert_eq!(registered[0], u16::from(b'"'));
            assert_eq!(registered[registered.len() - 2], u16::from(b'"'));
            set(false).unwrap();
            assert!(!enabled().unwrap());
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use anyhow::{Context, Result};
    use std::path::PathBuf;

    fn path() -> Result<PathBuf> {
        Ok(
            PathBuf::from(std::env::var_os("HOME").context("Missing user home directory")?)
                .join("Library/LaunchAgents/com.mytools.rust.plist"),
        )
    }
    fn definition() -> Result<String> {
        let exe = std::env::current_exe()?;
        let exe = exe.to_str().context("Startup path must be UTF-8")?;
        Ok(super::launch_agent(exe))
    }
    pub fn enabled() -> Result<bool> {
        match std::fs::read_to_string(path()?) {
            Ok(value) => Ok(value == definition()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    pub fn set(enabled: bool) -> Result<()> {
        let path = path()?;
        if enabled {
            std::fs::create_dir_all(path.parent().context("Missing LaunchAgents directory")?)?;
            std::fs::write(path, definition()?)?;
        } else if let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
        // launchd reads user LaunchAgents at the next login; do not launch a duplicate now.
        Ok(())
    }
}

#[cfg(any(target_os = "macos", test))]
fn launch_agent(exe: &str) -> String {
    let escaped = exe
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>com.mytools.rust</string>
<key>ProgramArguments</key><array><string>{escaped}</string></array>
<key>RunAtLoad</key><true/>
<key>AbandonProcessGroup</key><true/>
</dict></plist>
"#
    )
}

pub fn enabled() -> Result<bool> {
    platform::enabled()
}
pub fn set(enabled: bool) -> Result<()> {
    platform::set(enabled)
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub fn enabled() -> anyhow::Result<bool> {
        Ok(false)
    }
    pub fn set(_: bool) -> anyhow::Result<()> {
        anyhow::bail!("Login startup is not supported on this platform")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn launch_agent_keeps_the_executable_one_argument_and_escapes_xml() {
        let xml = super::launch_agent("/Users/a b/工具 & <x>/mytools");
        assert!(
            xml.contains("<array><string>/Users/a b/工具 &amp; &lt;x&gt;/mytools</string></array>")
        );
        assert!(xml.contains("<key>RunAtLoad</key><true/>"));
        assert!(!xml.contains("--show"));
    }
}
