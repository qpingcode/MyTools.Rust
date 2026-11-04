//! Reads an English word aloud with the operating system's own voices, offline.
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Speech {
    /// "auto" picks an installed US English voice; anything else is an exact voice name.
    pub voice: String,
    /// -10 (slowest) to 10 (fastest); 0 is the voice's normal speed.
    pub rate: i32,
}

impl Default for Speech {
    fn default() -> Self {
        Self {
            voice: "auto".into(),
            rate: 0,
        }
    }
}

impl Speech {
    fn named_voice(&self) -> Option<&str> {
        let voice = self.voice.trim();
        (!voice.is_empty() && !voice.eq_ignore_ascii_case("auto")).then_some(voice)
    }
}

#[cfg(windows)]
pub fn speak(text: &str, speech: &Speech) -> Result<(), String> {
    use windows::Win32::Media::Speech::{ISpVoice, SPF_IS_XML, SPF_PURGEBEFORESPEAK, SpVoice};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };
    use windows::core::HSTRING;
    // SAFETY: COM is initialised on this thread for the duration of the call and the voice
    // object is released before it is uninitialised.
    unsafe {
        let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| -> windows::core::Result<()> {
            let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;
            voice.SetRate(speech.rate.clamp(-10, 10))?;
            let markup = HSTRING::from(sapi_markup(text, speech));
            voice.Speak(
                &markup,
                (SPF_IS_XML.0 | SPF_PURGEBEFORESPEAK.0) as u32,
                None,
            )
        })();
        if initialised {
            CoUninitialize();
        }
        result.map_err(|error| error.message())
    }
}

/// SAPI XML that selects the voice: by name, or by US English (LCID 409) for "auto".
#[cfg(any(windows, test))]
fn sapi_markup(text: &str, speech: &Speech) -> String {
    let required = match speech.named_voice() {
        Some(name) => format!("Name={}", escape(name)),
        None => "Language=409".into(),
    };
    format!("<voice required=\"{required}\">{}</voice>", escape(text))
}

#[cfg(any(windows, test))]
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(target_os = "macos")]
pub fn speak(text: &str, speech: &Speech) -> Result<(), String> {
    let mut command = std::process::Command::new("say");
    if let Some(voice) = speech.named_voice() {
        command.args(["-v", voice]);
    }
    // `say` measures speed in words per minute; 175 is its default.
    let rate = 175 + speech.rate.clamp(-10, 10) * 15;
    let status = command
        .args(["-r", &rate.to_string(), "--", text])
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("say exited with {status}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn speak(_text: &str, _speech: &Speech) -> Result<(), String> {
    Err("speech is not supported on this platform".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_selects_english_or_a_named_voice_and_escapes_text() {
        assert_eq!(
            sapi_markup("rock & roll", &Speech::default()),
            "<voice required=\"Language=409\">rock &amp; roll</voice>"
        );
        let named = Speech {
            voice: "Microsoft Zira Desktop".into(),
            rate: 0,
        };
        assert_eq!(
            sapi_markup("hi", &named),
            "<voice required=\"Name=Microsoft Zira Desktop\">hi</voice>"
        );
    }

    /// Run with `--ignored` to hear the default voice say "hello".
    #[test]
    #[ignore]
    fn speaks_aloud() {
        speak("hello", &Speech::default()).unwrap();
    }
}
