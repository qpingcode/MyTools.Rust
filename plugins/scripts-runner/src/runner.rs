#[cfg(any(windows, test))]
use crate::store::Kind;
#[cfg(windows)]
use crate::store::Mode;
use crate::{
    i18n,
    store::{Script, Store},
};
use anyhow::Result;

pub fn run(store: &Store, script: &Script, admin: bool) -> Result<()> {
    script.validate()?;
    #[cfg(not(windows))]
    {
        let _ = (store, admin);
        anyhow::bail!(i18n::t("platform.unsupported"));
    }
    #[cfg(windows)]
    {
        let directory = store
            .root
            .join("runs")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory)?;
        let file = directory.join(format!("script.{}", script.kind.extension()));
        std::fs::write(&file, encode(script.kind, &script.content))?;
        let file = file.canonicalize()?;
        let home = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .unwrap_or(store.root.clone());
        let cwd = if script.directory.is_empty() {
            home
        } else {
            script.directory.clone().into()
        };
        let path = mytools_plugin_api::paths::display(&file);
        match script.mode {
            Mode::Window => launch(
                &executable(script.kind)?,
                &parameters(script.kind, &path),
                &cwd,
                admin,
                true,
            ),
            Mode::CloseOnSuccess => launch(
                &executable(Kind::Cmd)?,
                &close_parameters(
                    script.kind,
                    &executable(script.kind)?.to_string_lossy(),
                    &path,
                ),
                &cwd,
                admin,
                true,
            ),
            Mode::Background => {
                crate::background::prepare(&directory, script, &file, &cwd)?;
                let directory = mytools_plugin_api::paths::display(&directory.canonicalize()?);
                launch(
                    &std::env::current_exe()?,
                    &format!("--background \"{directory}\""),
                    &cwd,
                    admin,
                    false,
                )
            }
        }
    }
}

/// The system interpreter for a script type, never one found through `PATH`.
#[cfg(windows)]
pub fn executable(kind: Kind) -> Result<std::path::PathBuf> {
    let system = std::path::PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| anyhow::anyhow!(i18n::t("platform.unsupported")))?,
    )
    .join("System32");
    Ok(match kind {
        Kind::Ps1 => system.join("WindowsPowerShell/v1.0/powershell.exe"),
        Kind::Cmd => system.join("cmd.exe"),
    })
}

#[cfg(any(windows, test))]
fn encode(kind: Kind, content: &str) -> Vec<u8> {
    match kind {
        Kind::Ps1 => [vec![0xef, 0xbb, 0xbf], content.as_bytes().to_vec()].concat(),
        Kind::Cmd => format!(
            "@chcp 65001 >nul\r\n{}\r\n",
            content.replace("\r\n", "\n").replace('\n', "\r\n")
        )
        .into_bytes(),
    }
}

/// Arguments for a console that stays open after the script ends.
#[cfg(any(windows, test))]
fn parameters(kind: Kind, file: &str) -> String {
    match kind {
        Kind::Ps1 => format!("-NoLogo -NoProfile -NoExit -ExecutionPolicy Bypass -File \"{file}\""),
        Kind::Cmd => format!("/d /k \"\"{file}\"\""),
    }
}

/// `cmd.exe` arguments that run the script and pause only when it exits with an error,
/// so a successful run closes its console. `/c` drops the outer quotes and keeps the rest.
#[cfg(any(windows, test))]
fn close_parameters(kind: Kind, interpreter: &str, file: &str) -> String {
    let command = match kind {
        Kind::Ps1 => {
            format!("\"{interpreter}\" -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"{file}\"")
        }
        Kind::Cmd => format!("call \"{file}\""),
    };
    format!("/d /c \"{command} || pause\"")
}

#[cfg(windows)]
fn launch(
    executable: &std::path::Path,
    parameters: &str,
    directory: &std::path::Path,
    admin: bool,
    visible: bool,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL},
    };
    let wide = |value: &std::ffi::OsStr| value.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let executable = wide(executable.as_os_str());
    let directory = wide(directory.as_os_str());
    let parameters = wide(std::ffi::OsStr::new(parameters));
    let verb = wide(std::ffi::OsStr::new(if admin { "runas" } else { "open" }));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: if visible { SW_SHOWNORMAL } else { SW_HIDE },
        ..Default::default()
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(1223) {
            anyhow::bail!(i18n::t("run.cancelled"));
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_script_content_and_quotes_paths_without_inline_execution() {
        let content = "Write-Output '中文'\n# keep blank lines\n\n";
        assert_eq!(&encode(Kind::Ps1, content)[3..], content.as_bytes());
        assert_eq!(
            encode(Kind::Cmd, "echo 中文\n\necho done"),
            "@chcp 65001 >nul\r\necho 中文\r\n\r\necho done\r\n".as_bytes()
        );
        let path = r"C:\脚本 & tools\script.ps1";
        assert!(parameters(Kind::Ps1, path).ends_with(&format!("-File \"{path}\"")));
        assert_eq!(
            parameters(Kind::Cmd, r"C:\scripts\file.cmd"),
            r#"/d /k ""C:\scripts\file.cmd"""#
        );
    }
    #[test]
    fn close_on_success_pauses_only_after_a_failure() {
        assert_eq!(
            close_parameters(Kind::Cmd, "cmd.exe", r"C:\a b\file.cmd"),
            r#"/d /c "call "C:\a b\file.cmd" || pause""#
        );
        assert_eq!(
            close_parameters(Kind::Ps1, r"C:\ps.exe", r"C:\a\file.ps1"),
            r#"/d /c ""C:\ps.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "C:\a\file.ps1" || pause""#
        );
    }
}
