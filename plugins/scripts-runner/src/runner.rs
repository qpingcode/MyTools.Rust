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
        let home = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .unwrap_or(store.root.clone());
        let cwd = if script.directory.is_empty() {
            home
        } else {
            script.directory.clone().into()
        };
        let file = directory.join(format!("script.{}", script.kind.extension()));
        let location = mytools_plugin_api::paths::display(&cwd);
        std::fs::write(&file, encode(script.kind, &location, &script.content))?;
        let file = file.canonicalize()?;
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

/// The interpreter for a script type at its fixed install location, never one found through
/// `PATH`, so an elevated run cannot pick up a planted copy. A missing PowerShell 7 is not
/// replaced by Windows PowerShell, since scripts written for one may fail in the other.
#[cfg(windows)]
pub fn executable(kind: Kind) -> Result<std::path::PathBuf> {
    if kind == Kind::Pwsh {
        return pwsh().ok_or_else(|| anyhow::anyhow!(i18n::t("pwsh.missing")));
    }
    let system = std::path::PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| anyhow::anyhow!(i18n::t("platform.unsupported")))?,
    )
    .join("System32");
    Ok(match kind {
        Kind::Ps1 | Kind::Pwsh => system.join("WindowsPowerShell/v1.0/powershell.exe"),
        Kind::Cmd => system.join("cmd.exe"),
    })
}

/// PowerShell 7 where its installer puts it.
#[cfg(windows)]
fn pwsh() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os("ProgramFiles")?)
        .join(r"PowerShell\7\pwsh.exe");
    path.is_file().then_some(path)
}

/// The type a new script starts with: PowerShell 7 when it is installed.
pub fn preferred_kind() -> Kind {
    #[cfg(windows)]
    if pwsh().is_some() {
        return Kind::Pwsh;
    }
    Kind::Ps1
}

/// Writes the script with a first line that enters `directory`. Windows starts an elevated
/// `powershell.exe` or `cmd.exe` in `System32` whatever working directory it is given, so the
/// script sets its own; the line stops the script when the directory cannot be entered.
#[cfg(any(windows, test))]
fn encode(kind: Kind, directory: &str, content: &str) -> Vec<u8> {
    match kind {
        Kind::Ps1 | Kind::Pwsh => format!(
            "\u{feff}Set-Location -LiteralPath '{}' -ErrorAction Stop\n{content}",
            powershell_literal(directory)
        )
        .into_bytes(),
        Kind::Cmd => format!(
            "@chcp 65001 >nul\r\n@cd /d \"{}\" || exit /b 1\r\n{}\r\n",
            directory.replace('%', "%%"),
            content.replace("\r\n", "\n").replace('\n', "\r\n")
        )
        .into_bytes(),
    }
}

/// Escapes text for a single-quoted PowerShell string, which also ends at typographic quotes.
#[cfg(any(windows, test))]
pub(crate) fn powershell_literal(text: &str) -> String {
    text.chars()
        .flat_map(|ch| match ch {
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' => vec![ch, ch],
            _ => vec![ch],
        })
        .collect()
}

/// Arguments for a console that stays open after the script ends.
#[cfg(any(windows, test))]
fn parameters(kind: Kind, file: &str) -> String {
    match kind {
        Kind::Ps1 | Kind::Pwsh => format!("-NoLogo -NoProfile -NoExit -ExecutionPolicy Bypass -File \"{file}\""),
        Kind::Cmd => format!("/d /k \"\"{file}\"\""),
    }
}

/// `cmd.exe` arguments that run the script and pause only when it exits with an error,
/// so a successful run closes its console. `/c` drops the outer quotes and keeps the rest.
#[cfg(any(windows, test))]
fn close_parameters(kind: Kind, interpreter: &str, file: &str) -> String {
    let command = match kind {
        Kind::Ps1 | Kind::Pwsh => {
            format!("\"{interpreter}\" -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"{file}\"")
        }
        Kind::Cmd => format!("call \"{file}\""),
    };
    format!("/d /c \"{command} || pause\"")
}

#[cfg(windows)]
pub(crate) fn launch(
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
        assert_eq!(
            encode(Kind::Ps1, r"C:\中文 dir", content),
            format!("\u{feff}Set-Location -LiteralPath 'C:\\中文 dir' -ErrorAction Stop\n{content}")
                .as_bytes()
        );
        assert_eq!(
            encode(Kind::Cmd, r"C:\a b", "echo 中文\n\necho done"),
            "@chcp 65001 >nul\r\n@cd /d \"C:\\a b\" || exit /b 1\r\necho 中文\r\n\r\necho done\r\n"
                .as_bytes()
        );
        let path = r"C:\脚本 & tools\script.ps1";
        assert!(parameters(Kind::Ps1, path).ends_with(&format!("-File \"{path}\"")));
        assert_eq!(
            parameters(Kind::Cmd, r"C:\scripts\file.cmd"),
            r#"/d /k ""C:\scripts\file.cmd"""#
        );
    }
    #[test]
    fn escapes_quotes_and_percent_signs_in_the_directory() {
        assert_eq!(powershell_literal("C:\\it's ‘x’"), "C:\\it''s ‘‘x’’");
        assert!(
            String::from_utf8(encode(Kind::Cmd, r"C:\100%\a", ""))
                .unwrap()
                .contains(r#"@cd /d "C:\100%%\a""#)
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
