//! Finds, closes and starts Google Chrome.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Feature switches that skip Glic's country and locale checks and turn its UI on.
pub fn glic_arguments(profile: &str) -> Vec<String> {
    vec![
        format!("--profile-directory={profile}"),
        "--glic-dev".into(),
        "--disable-features=GlicCountryFiltering,GlicLocaleFiltering,GlicUseSessionCountryForFiltering".into(),
        "--enable-features=Glic,GlicButtonAltLabel".into(),
    ]
}

pub fn user_data_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MYTOOLS_CHROME_DIR") {
        return Some(path.into());
    }
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("Google/Chrome/User Data"));
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME")
        .map(|root| PathBuf::from(root).join("Library/Application Support/Google/Chrome"));
    #[cfg(not(any(windows, target_os = "macos")))]
    std::env::var_os("HOME").map(|root| PathBuf::from(root).join(".config/google-chrome"))
}

pub fn executable() -> Option<PathBuf> {
    #[cfg(windows)]
    return ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(r"Google\Chrome\Application\chrome.exe"))
        .find(|path| path.is_file());
    #[cfg(target_os = "macos")]
    return Some(PathBuf::from(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ))
    .filter(|path| path.is_file());
    #[cfg(not(any(windows, target_os = "macos")))]
    Some(PathBuf::from("google-chrome"))
}

#[cfg(windows)]
pub fn is_running() -> bool {
    let output = hidden("tasklist")
        .args(["/FI", "IMAGENAME eq chrome.exe", "/FO", "CSV", "/NH"])
        .output();
    output.is_ok_and(|output| {
        String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("\"chrome.exe\"")
    })
}

#[cfg(not(windows))]
pub fn is_running() -> bool {
    hidden("pgrep")
        .args(["-x", PROCESS])
        .status()
        .is_ok_and(|status| status.success())
}

/// Asks Chrome to close its windows, so it saves its session normally, and ends it
/// forcibly when it has not exited after a few seconds (e.g. a "Leave site?" prompt).
pub fn close() -> Result<(), String> {
    request_exit(false);
    if wait_for_exit(Duration::from_secs(5)) {
        return Ok(());
    }
    request_exit(true);
    if wait_for_exit(Duration::from_secs(10)) {
        Ok(())
    } else {
        Err("Chrome is still running".into())
    }
}

#[cfg(windows)]
fn request_exit(force: bool) {
    let mut command = hidden("taskkill");
    command.args(["/IM", "chrome.exe"]);
    if force {
        command.args(["/T", "/F"]);
    }
    let _ = command.status();
}

#[cfg(not(windows))]
const PROCESS: &str = if cfg!(target_os = "macos") {
    "Google Chrome"
} else {
    "chrome"
};

#[cfg(not(windows))]
fn request_exit(force: bool) {
    let signal = if force { "-KILL" } else { "-TERM" };
    let _ = hidden("pkill").args([signal, "-x", PROCESS]).status();
}

fn wait_for_exit(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !is_running() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Starts Chrome detached from this plugin: it must not inherit the plugin's protocol pipes
/// or end when MyTools restarts the plugin.
pub fn start(executable: &Path, arguments: &[String]) -> Result<(), String> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    command.spawn().map(drop).map_err(|error| error.to_string())
}

fn hidden(program: &str) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_open_the_profile_and_bypass_glic_filters() {
        let arguments = glic_arguments("Profile 4").join(" ");
        assert!(arguments.starts_with("--profile-directory=Profile 4 --glic-dev"));
        assert!(arguments.contains("GlicCountryFiltering,GlicLocaleFiltering"));
        assert!(arguments.ends_with("--enable-features=Glic,GlicButtonAltLabel"));
    }
}
