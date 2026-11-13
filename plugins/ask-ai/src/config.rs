use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub base_url: String,
    pub model: String,
    /// Empty: [`DEFAULT_PROMPT`].
    pub system_prompt: String,
    pub max_tokens: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: mytools_ai::DEEPSEEK_BASE_URL.into(),
            model: mytools_ai::DEEPSEEK_DEFAULT_MODEL.into(),
            system_prompt: String::new(),
            max_tokens: 8192,
        }
    }
}

const DEFAULT_PROMPT: &str = "You are a helpful assistant inside MyTools, a desktop launcher. \
Answer in the language of the user's latest message. Be concise and direct; use Markdown \
(lists, inline code, fenced code blocks with a language) when it helps.";

impl Config {
    /// `config.json` next to the executable, or `MYTOOLS_ASK_AI_CONFIG`; defaults when absent.
    pub fn load() -> Result<Self, String> {
        let path = std::env::var_os("MYTOOLS_ASK_AI_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .unwrap_or_default()
                    .with_file_name("config.json")
            });
        match std::fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                serde_json::from_str(&mytools_ai::jsonc::strip_comments(&text))
                    .map_err(|e| e.to_string())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn provider(&self) -> mytools_ai::Provider {
        mytools_ai::Provider::deepseek(&self.base_url, &self.model)
    }

    pub fn system_prompt(&self) -> &str {
        match self.system_prompt.trim() {
            "" => DEFAULT_PROMPT,
            prompt => prompt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_documents_the_defaults() {
        let text = include_str!("../config.example.json");
        let config: Config =
            serde_json::from_str(&mytools_ai::jsonc::strip_comments(text)).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.system_prompt(), DEFAULT_PROMPT);
    }
}
