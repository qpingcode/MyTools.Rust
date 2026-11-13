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
    /// Models offered by Ctrl+K besides `model`.
    pub models: Vec<String>,
    /// Offer the built-in web search to the model.
    pub web_search: bool,
    /// How many conversations the history keeps; 0 keeps none.
    pub history: usize,
    pub search: Search,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: mytools_ai::DEEPSEEK_BASE_URL.into(),
            model: mytools_ai::DEEPSEEK_DEFAULT_MODEL.into(),
            system_prompt: String::new(),
            max_tokens: 8192,
            models: Vec::new(),
            web_search: true,
            history: 50,
            search: Search::default(),
        }
    }
}

/// The built-in web search.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Search {
    /// Tried in order: `tavily` (needs `TAVILY_API_KEY`), `bing`, `duckduckgo`.
    pub providers: Vec<String>,
    pub max_results: usize,
    pub bing: Bing,
    pub duckduckgo: DuckDuckGo,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            providers: vec!["tavily".into(), "bing".into()],
            max_results: 5,
            bing: Bing::default(),
            duckduckgo: DuckDuckGo::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Bing {
    pub host: String,
    pub market: String,
}

impl Default for Bing {
    fn default() -> Self {
        Self {
            host: "www.bing.com".into(),
            market: "zh-CN".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DuckDuckGo {
    pub region: String,
}

impl Default for DuckDuckGo {
    fn default() -> Self {
        Self {
            region: "cn-zh".into(),
        }
    }
}

const DEFAULT_PROMPT: &str = "You are a helpful assistant inside MyTools, a desktop launcher. \
Answer in the language of the user's latest message. Be concise and direct; use Markdown \
(lists, inline code, fenced code blocks with a language) when it helps.";

/// A JSONC file next to the executable, or the one an environment variable names;
/// defaults when it does not exist.
pub fn read<T: serde::de::DeserializeOwned + Default>(
    variable: &str,
    name: &str,
) -> Result<T, String> {
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name(name)
        });
    match std::fs::read(&path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            serde_json::from_str(&mytools_ai::jsonc::strip_comments(&text))
                .map_err(|e| format!("{}: {e}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

impl Config {
    /// `config.json` next to the executable, or `MYTOOLS_ASK_AI_CONFIG`.
    pub fn load() -> Result<Self, String> {
        read("MYTOOLS_ASK_AI_CONFIG", "config.json")
    }

    /// The endpoint with `model`, or the configured one.
    pub fn provider(&self, model: Option<&str>) -> mytools_ai::Provider {
        let mut provider = mytools_ai::Provider::deepseek(&self.base_url, &self.model);
        if let Some(model) = model {
            provider.model = model.into();
        }
        provider
    }

    /// The configured model first, then the others to pick from.
    pub fn all_models(&self) -> Vec<String> {
        let mut models = vec![self.model.clone()];
        for model in &self.models {
            if !models.contains(model) {
                models.push(model.clone());
            }
        }
        models
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
