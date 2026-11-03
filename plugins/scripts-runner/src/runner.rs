#[cfg(any(windows, test))]
use crate::store::Kind;
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
        let system = std::path::PathBuf::from(
            std::env::var_os("SystemRoot")
                .ok_or_else(|| anyhow::anyhow!(i18n::t("platform.unsupported")))?,
        )
        .join("System32");
        let executable = match script.kind {
            Kind::Ps1 => system.join("WindowsPowerShell/v1.0/powershell.exe"),
            Kind::Cmd => system.join("cmd.exe"),
        };
        let path = mytools_plugin_api::paths::display(&file);
        let parameters = parameters(script.kind, &path);
        launch(&executable, &parameters, &cwd, admin)
    }
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

#[cfg(any(windows, test))]
fn parameters(kind: Kind, file: &str) -> String {
    match kind {
        Kind::Ps1 => format!("-NoLogo -NoProfile -NoExit -ExecutionPolicy Bypass -File \"{file}\""),
        Kind::Cmd => format!("/d /k \"\"{file}\"\""),
    }
}

#[cfg(windows)]
fn launch(
    executable: &std::path::Path,
    parameters: &str,
    directory: &std::path::Path,
    admin: bool,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::{
        Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
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
        nShow: SW_SHOWNORMAL,
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
}
