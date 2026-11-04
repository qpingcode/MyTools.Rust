//! Locates installed IDEs and starts them detached from the plugin's stdio pipes.
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ide {
    Rider,
    VsCode,
    VisualStudio,
}

impl Ide {
    pub fn from_command(command: &str) -> Option<Self> {
        match command {
            "rider" => Some(Self::Rider),
            "vscode" => Some(Self::VsCode),
            "visualstudio" => Some(Self::VisualStudio),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Rider => "rider",
            Self::VsCode => "vscode",
            Self::VisualStudio => "visualstudio",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Rider => "Rider",
            Self::VsCode => "VS Code",
            Self::VisualStudio => "Visual Studio",
        }
    }
}

/// Optional `config.json` beside the plugin overriding auto-detected executables.
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct Config {
    rider: String,
    vscode: String,
    visualstudio: String,
}

impl Config {
    pub fn load() -> Self {
        std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name("config.json"))
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn configured(&self, ide: Ide) -> Option<PathBuf> {
        let value = match ide {
            Ide::Rider => &self.rider,
            Ide::VsCode => &self.vscode,
            Ide::VisualStudio => &self.visualstudio,
        };
        let path = PathBuf::from(value.trim());
        (path.is_absolute() && path.is_file()).then_some(path)
    }
}

pub fn locate(ide: Ide, config: &Config) -> Option<PathBuf> {
    config.configured(ide).or_else(|| match ide {
        Ide::Rider => rider(),
        Ide::VsCode => vscode(),
        Ide::VisualStudio => visual_studio(),
    })
}

fn env_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from)
}

/// Executables matching `<root>/<any folder whose name contains marker>/<relative>`.
fn installs(root: Option<PathBuf>, marker: &str, relative: &str) -> Vec<PathBuf> {
    let Some(entries) = root.and_then(|root| std::fs::read_dir(root).ok()) else {
        return vec![];
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains(marker)
        })
        .map(|entry| entry.path().join(relative))
        .filter(|path| path.is_file())
        .collect()
}

fn newest(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().max_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    })
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|path| path.is_file())
}

fn rider() -> Option<PathBuf> {
    let local = env_path("LOCALAPPDATA");
    // JetBrains Toolbox writes a launcher script that names the active installation.
    let toolbox = local
        .as_ref()
        .map(|local| local.join(r"JetBrains\Toolbox\scripts\rider.cmd"))
        .and_then(|script| std::fs::read_to_string(script).ok())
        .and_then(|script| toolbox_target(&script));
    toolbox.or_else(|| {
        let relative = r"bin\rider64.exe";
        let mut candidates = installs(
            local.as_ref().map(|l| l.join(r"JetBrains\Installations")),
            "rider",
            relative,
        );
        candidates.extend(installs(
            local.as_ref().map(|l| l.join("Programs")),
            "rider",
            relative,
        ));
        candidates.extend(installs(
            env_path("ProgramFiles").map(|p| p.join("JetBrains")),
            "rider",
            relative,
        ));
        newest(candidates).or_else(|| on_path(&["rider64.exe"]))
    })
}

/// Extracts `X:\...\rider64.exe` from the Toolbox `start "" ... rider64.exe %args%` line.
fn toolbox_target(script: &str) -> Option<PathBuf> {
    let lower = script.to_ascii_lowercase();
    let end = lower.find("rider64.exe")? + "rider64.exe".len();
    let start = lower[..end].rfind(":\\")?.checked_sub(1)?;
    let path = PathBuf::from(&script[start..end]);
    path.is_file().then_some(path)
}

fn vscode() -> Option<PathBuf> {
    [
        env_path("LOCALAPPDATA").map(|p| p.join(r"Programs\Microsoft VS Code\Code.exe")),
        env_path("ProgramFiles").map(|p| p.join(r"Microsoft VS Code\Code.exe")),
        env_path("ProgramFiles(x86)").map(|p| p.join(r"Microsoft VS Code\Code.exe")),
    ]
    .into_iter()
    .flatten()
    .find(|path| path.is_file())
    .or_else(|| on_path(&["Code.exe"]))
}

fn visual_studio() -> Option<PathBuf> {
    let vswhere =
        env_path("ProgramFiles(x86)")?.join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    let mut command = Command::new(vswhere);
    // `-latest` may pick another VS-shell product such as SSMS, so list all and filter.
    command
        .args([
            "-all",
            "-prerelease",
            "-products",
            "*",
            "-format",
            "json",
            "-utf8",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    let output = command.output().ok()?;
    newest_devenv(&String::from_utf8_lossy(&output.stdout))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VsInstance {
    #[serde(default)]
    product_path: PathBuf,
    #[serde(default)]
    installation_version: String,
}

/// Picks the highest-version instance whose product is devenv.exe.
fn newest_devenv(json: &str) -> Option<PathBuf> {
    let instances: Vec<VsInstance> = serde_json::from_str(json).ok()?;
    instances
        .into_iter()
        .filter(|instance| {
            instance
                .product_path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("devenv.exe"))
                && instance.product_path.is_file()
        })
        .max_by_key(|instance| {
            instance
                .installation_version
                .split('.')
                .map(|part| part.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .map(|instance| instance.product_path)
}

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let _ = command;
}

/// Starts the IDE without inheriting the plugin's stdin/stdout, which carry the host protocol.
pub fn launch(
    executable: &Path,
    arguments: &[&Path],
    working_directory: &Path,
) -> std::io::Result<()> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: the IDE outlives the plugin.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    command.spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        for ide in [Ide::Rider, Ide::VsCode, Ide::VisualStudio] {
            assert_eq!(Ide::from_command(ide.command()), Some(ide));
        }
        assert_eq!(Ide::from_command("other"), None);
    }

    #[test]
    fn newest_devenv_ignores_other_vs_shell_products() {
        let json = r#"[
            {"productPath":"C:\missing\SSMS.exe","installationVersion":"22.0.1"},
            {"productPath":"C:\missing\devenv.exe","installationVersion":"17.9.0"}
        ]"#;
        // Paths do not exist on the test machine, so nothing qualifies.
        assert_eq!(newest_devenv(json), None);
        assert_eq!(newest_devenv("not json"), None);
    }

    /// Run with `--ignored --nocapture` to see what this machine resolves.
    #[test]
    #[ignore]
    fn prints_detected_ides() {
        let config = Config::default();
        for ide in [Ide::Rider, Ide::VsCode, Ide::VisualStudio] {
            println!("{:?} -> {:?}", ide, locate(ide, &config));
        }
    }

    #[test]
    fn toolbox_script_target_requires_existing_file() {
        let script = "start \"\" %waitarg% C:\\missing\\Rider\\bin\\rider64.exe %intellij_args%";
        assert_eq!(toolbox_target(script), None);
    }
}
