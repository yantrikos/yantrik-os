//! What the reach tests share: the full registry, and reading the sources a tool is made of.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use yantrik_companion::config::CompanionConfig;
use yantrik_companion::tools::build_registry;
use yantrik_companion_core::permission::PermissionLevel;

pub fn config_with_everything_on() -> CompanionConfig {
    serde_yaml::from_str(
        r#"
llm: { backend: ollama, api_base_url: "http://localhost:11434/v1", api_model: "m" }
home_assistant: { enabled: true, base_url: "http://ha.local", token: "t" }
telegram: { enabled: true, bot_token: "t", chat_id: "1" }
whatsapp: { enabled: true, phone_number_id: "p", access_token: "t" }
email: { enabled: true, accounts: [ { name: "n", email: "a@b.c", provider: "gmail" } ] }
"#,
    )
    .expect("a config with every optional tool on")
}

pub fn tools() -> Vec<(&'static str, &'static str)> {
    static TOOLS: std::sync::OnceLock<Vec<(&'static str, &'static str)>> = std::sync::OnceLock::new();
    TOOLS
        .get_or_init(|| {
            build_registry(&config_with_everything_on())
                .list_metadata(PermissionLevel::Dangerous)
                .into_iter()
                .map(|m| (m.name, m.category))
                .collect()
        })
        .clone()
}

pub fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The tool names a file registers: the string literal after each `fn name(&self) -> &'static str`.
pub fn tool_names(code: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("fn name(&self) -> &'static str") {
        rest = &rest[at + 30..];
        let window = &rest[..rest.len().min(120)];
        if let Some(open) = window.find('"') {
            if let Some(len) = window[open + 1..].find('"') {
                names.push(window[open + 1..open + 1 + len].to_string());
            }
        }
    }
    names
}
