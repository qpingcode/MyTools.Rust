//! Calling chat models for plugins such as translator and Ask AI: an OpenAI-compatible
//! `/chat/completions` client with whole and streamed replies, DeepSeek defaults, and the
//! environment and JSONC helpers their configuration needs.
mod client;
pub mod environment;
pub mod jsonc;
pub mod mcp;
mod stream;

pub use client::{Provider, complete, describe};
pub use stream::{Delta, Stopped, ToolCall, stream};

/// The variable holding the DeepSeek API key; configuration files never carry keys.
pub const DEEPSEEK_API_KEY: &str = "DEEPSEEK_API_KEY";
/// Overrides the configured model, e.g. to try another one without editing a file.
pub const DEEPSEEK_MODEL: &str = "DEEPSEEK_MODEL";
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-flash";
