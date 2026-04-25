use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub whisper_url:   String,
    pub ollama_url:    String,
    pub whisper_model: String,
    pub llm_model:     String,
    #[serde(default)]
    pub hotkey:        Option<String>,
    pub prompt:        String,
}

pub fn parse_config(raw: &str) -> Result<Config> {
    toml::from_str(raw).map_err(|e| anyhow::anyhow!("invalid config.toml: {e}"))
}

pub fn load_config() -> Result<Config> {
    let raw = std::fs::read_to_string("config.toml")
        .map_err(|e| anyhow::anyhow!("cannot read config.toml: {e}"))?;
    parse_config(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOML: &str = r#"
        whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
        ollama_url    = "http://localhost:11434/api/generate"
        whisper_model = "whisper-large-turbo"
        llm_model     = "qwen2.5:7b-instruct"
        prompt        = "Fix grammar.\n\n---\n"
    "#;

    #[test]
    fn valid_toml_parses() {
        let cfg = parse_config(VALID_TOML).unwrap();
        assert_eq!(cfg.llm_model, "qwen2.5:7b-instruct");
        assert_eq!(cfg.whisper_model, "whisper-large-turbo");
        assert_eq!(cfg.whisper_url, "http://localhost:8080/v1/audio/transcriptions");
        assert_eq!(cfg.ollama_url, "http://localhost:11434/api/generate");
        assert_eq!(cfg.prompt, "Fix grammar.\n\n---\n");
        assert_eq!(cfg.hotkey, None);
    }

    #[test]
    fn hotkey_field_parses() {
        let raw = r#"
            whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
            ollama_url    = "http://localhost:11434/api/generate"
            whisper_model = "whisper-large-turbo"
            llm_model     = "qwen2.5:7b-instruct"
            hotkey        = "Alt+W"
            prompt        = "Fix.\n"
        "#;
        let cfg = parse_config(raw).unwrap();
        assert_eq!(cfg.hotkey.as_deref(), Some("Alt+W"));
    }

    #[test]
    fn missing_required_field_fails() {
        let raw = r#"
            whisper_url = "http://localhost:8080"
            ollama_url  = "http://localhost:11434"
        "#;
        assert!(parse_config(raw).is_err());
    }

    #[test]
    fn invalid_toml_fails() {
        assert!(parse_config("this is not toml :::").is_err());
    }
}
