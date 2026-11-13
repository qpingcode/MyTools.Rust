//! Runs a command typed after the plugin's alias, e.g. `> podman ps`, in a terminal.
use crate::i18n;
use anyhow::{Result, ensure};

/// Commands are passed on the command line, which Windows limits to 32767 characters.
const MAX_COMMAND_BYTES: usize = 4096;

/// Runs `command` in a PowerShell window that stays open, with the user's profile loaded as
/// in a terminal they opened themselves. Windows shows it in the default terminal app.
pub fn run(command: &str, admin: bool) -> Result<()> {
    let command = command.trim();
    ensure!(
        !command.is_empty() && command.len() <= MAX_COMMAND_BYTES && !command.contains('\0'),
        i18n::t("validation.command")
    );
    #[cfg(not(windows))]
    {
        let _ = admin;
        anyhow::bail!(i18n::t("platform.unsupported"));
    }
    #[cfg(windows)]
    {
        let home = std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        crate::runner::launch(&shell()?, &parameters(command), &home, admin, true)
    }
}

/// PowerShell 7 when it is on `PATH`, like the commands typed into it; otherwise the
/// Windows PowerShell that ships with the system.
#[cfg(windows)]
fn shell() -> Result<std::path::PathBuf> {
    let found = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join("pwsh.exe"))
            .find(|path| path.is_file())
    });
    match found {
        Some(path) => Ok(path),
        None => crate::runner::executable(crate::store::Kind::Ps1),
    }
}

/// `-EncodedCommand` takes Base64 of UTF-16LE, so quotes, `;` and `&` in the command reach
/// PowerShell unchanged instead of being parsed as part of the command line.
#[cfg(any(windows, test))]
fn parameters(command: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    format!("-NoLogo -NoExit -EncodedCommand {encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn encodes_the_command_unchanged() {
        let command = r#"podman ps; echo "中文 & done""#;
        let parameters = parameters(command);
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
        assert_eq!(String::from_utf16(&units).unwrap(), command);
    }

    #[test]
    fn rejects_empty_or_oversized_commands() {
        assert!(run("  ", false).is_err());
        assert!(run(&"x".repeat(MAX_COMMAND_BYTES + 1), false).is_err());
    }
}
