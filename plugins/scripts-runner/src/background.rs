//! Background runs: a hidden copy of this plugin (`--background <run dir>`) runs the script
//! without a console, keeps its output next to it and reports only a failure.
use crate::{
    i18n,
    store::{Kind, Script},
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const JOB: &str = "job.json";
const LOG: &str = "output.log";
/// How much of the output a failure dialog shows.
const TAIL_LINES: usize = 15;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Job {
    name: String,
    kind: Kind,
    file: PathBuf,
    directory: PathBuf,
}

/// Writes what the hidden runner needs, so its command line only carries the run directory.
pub fn prepare(run: &Path, script: &Script, file: &Path, directory: &Path) -> Result<()> {
    let job = Job {
        name: script.name.clone(),
        kind: script.kind,
        file: file.into(),
        directory: directory.into(),
    };
    std::fs::write(run.join(JOB), serde_json::to_vec(&job)?)?;
    Ok(())
}

/// Entry point of the hidden runner.
pub fn run(run: &Path) -> Result<()> {
    let job: Job = serde_json::from_slice(&std::fs::read(run.join(JOB))?)?;
    let log = run.join(LOG);
    let failure = match execute(&job) {
        Ok((code, output)) => {
            std::fs::write(&log, &output)?;
            (code != 0).then(|| {
                i18n::format(
                    "background.failed",
                    &[
                        ("name", &job.name),
                        ("code", &code.to_string()),
                        ("output", &tail(&output)),
                        ("log", &mytools_plugin_api::paths::display(&log)),
                    ],
                )
            })
        }
        Err(error) => Some(i18n::format(
            "background.error",
            &[("name", &job.name), ("error", &error.to_string())],
        )),
    };
    if let Some(message) = failure {
        alert(&i18n::t("name"), &message);
    }
    Ok(())
}

/// Runs the script hidden with no input, returning its exit code and combined output.
#[cfg(windows)]
fn execute(job: &Job) -> Result<(i32, String)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let file = mytools_plugin_api::paths::display(&job.file);
    // Both run under `cmd`: PowerShell writes in the console code page, so switching the
    // hidden console to UTF-8 first keeps non-ASCII output intact without editing the script.
    let script = match job.kind {
        // Prompts fail instead of waiting for input nobody can see.
        Kind::Ps1 => format!(
            "chcp 65001 >nul & \"{}\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{file}\"",
            crate::runner::executable(Kind::Ps1)?.display()
        ),
        Kind::Cmd => format!("\"{file}\""),
    };
    let mut command = std::process::Command::new(crate::runner::executable(Kind::Cmd)?);
    command.raw_arg(format!("/d /c \"{script}\""));
    let output = command
        .current_dir(&job.directory)
        .stdin(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    let text = [decode(&output.stdout), decode(&output.stderr)]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok((output.status.code().unwrap_or(1), text))
}
#[cfg(not(windows))]
fn execute(_: &Job) -> Result<(i32, String)> {
    anyhow::bail!(i18n::t("platform.unsupported"))
}

/// CMD runs in UTF-8 (`chcp 65001`); Windows PowerShell writes in the console's OEM code page.
#[cfg(windows)]
fn decode(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.into();
    }
    use windows_sys::Win32::Globalization::{CP_OEMCP, MultiByteToWideChar};
    let Ok(length) = i32::try_from(bytes.len()) else {
        return String::from_utf8_lossy(bytes).into();
    };
    // SAFETY: the first call only measures; the second writes at most `size` units.
    unsafe {
        let size =
            MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), length, std::ptr::null_mut(), 0);
        if size <= 0 {
            return String::from_utf8_lossy(bytes).into();
        }
        let mut wide = vec![0u16; size as usize];
        let written =
            MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), length, wide.as_mut_ptr(), size);
        String::from_utf16_lossy(&wide[..written.max(0) as usize])
    }
}

/// The last non-empty lines of the output.
fn tail(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n")
}

#[cfg(windows)]
fn alert(title: &str, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (title, message) = (wide(title), wide(message));
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}
#[cfg(not(windows))]
fn alert(_: &str, _: &str) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn job_roundtrips_and_tail_keeps_the_last_lines() {
        let run = std::env::temp_dir().join(format!("scripts-job-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&run).unwrap();
        let script = Script {
            id: uuid::Uuid::new_v4().to_string(),
            name: "构建".into(),
            alias: String::new(),
            kind: Kind::Cmd,
            content: "echo hi".into(),
            directory: String::new(),
            mode: crate::store::Mode::Background,
        };
        prepare(
            &run,
            &script,
            Path::new(r"C:\run\script.cmd"),
            Path::new(r"C:\work"),
        )
        .unwrap();
        let job: Job = serde_json::from_slice(&std::fs::read(run.join(JOB)).unwrap()).unwrap();
        assert_eq!(job.name, "构建");
        assert_eq!(job.directory, PathBuf::from(r"C:\work"));
        std::fs::remove_dir_all(run).unwrap();
        let output = (1..=20)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n\n");
        assert_eq!(tail(&output).lines().count(), TAIL_LINES);
        assert!(tail(&output).starts_with("6\n") && tail(&output).ends_with("20"));
    }
}
