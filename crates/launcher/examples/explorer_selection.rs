#[cfg(windows)]
#[path = "../src/desktop/explorer_windows.rs"]
mod explorer;

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    probe::run()
}

#[cfg(windows)]
mod probe {
    use std::{thread, time::Duration};

    use super::explorer;
    use mytools_plugin_api::i18n::{Catalog, language};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    const DELAY_SECONDS: u64 = 5;

    pub fn run() -> anyhow::Result<()> {
        let english = include_str!("../locales/en.json");
        let text = Catalog::from_json(
            if language() == "zh-CN" {
                include_str!("../locales/zh-CN.json")
            } else {
                english
            },
            english,
        )?;
        println!(
            "{}",
            text.format(
                "explorer_probe.prompt",
                &[("count", &DELAY_SECONDS.to_string())],
            )
        );
        thread::sleep(Duration::from_secs(DELAY_SECONDS));

        let foreground = unsafe { GetForegroundWindow() };
        let Some(paths) =
            explorer::selected_files(foreground, text.get("explorer_probe.ambiguous"))?
        else {
            println!("{}", text.get("explorer_probe.no_window"));
            return Ok(());
        };
        println!(
            "{}",
            text.format(
                "explorer_probe.result",
                &[("count", &paths.len().to_string())]
            )
        );
        for path in paths {
            println!("{}", path.display());
        }
        Ok(())
    }
}
