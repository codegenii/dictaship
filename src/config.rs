use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct ModeConfig {
    pub name:   String,
    pub prompt: String,
}

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub whisper_url:   String,
    pub ollama_url:    String,
    pub whisper_model: String,
    pub llm_model:     String,
    #[serde(default)]
    pub hotkey:        Option<String>,
    pub prompt:        String,
    #[serde(default)]
    pub distill_mode:  Option<String>,
    #[serde(default)]
    pub modes:         Vec<ModeConfig>,
    #[serde(default)]
    pub settings_w:    Option<u32>,
    #[serde(default)]
    pub settings_h:    Option<u32>,
}

impl Config {
    pub fn active_prompt(&self, mode_name: &str) -> &str {
        self.modes.iter()
            .find(|m| m.name == mode_name)
            .map(|m| m.prompt.as_str())
            .unwrap_or(&self.prompt)
    }
}

pub fn default_modes(legacy_prompt: &str) -> Vec<ModeConfig> {
    vec![
        ModeConfig {
            name:   "Distill".to_string(),
            prompt: legacy_prompt.to_string(),
        },
        ModeConfig {
            name:   "Translate to English".to_string(),
            prompt: "Translate the following speech to English. \
                     Output ONLY the translation, no preamble.\n\n---\n".to_string(),
        },
        ModeConfig {
            name:   "Clean text".to_string(),
            prompt: "Remove all filler words, interjections, and hesitations. \
                     Fix punctuation. Output ONLY the cleaned text.\n\n---\n".to_string(),
        },
    ]
}

pub fn parse_config(raw: &str) -> Result<Config> {
    toml::from_str(raw).map_err(|e| anyhow::anyhow!("invalid config.toml: {e}"))
}

pub fn load_config() -> Result<Config> {
    let raw = std::fs::read_to_string("config.toml")
        .map_err(|e| anyhow::anyhow!("cannot read config.toml: {e}"))?;
    let mut cfg = parse_config(&raw)?;
    if cfg.modes.is_empty() {
        cfg.modes = default_modes(&cfg.prompt);
    }
    Ok(cfg)
}

fn reserialize(path: &str, f: impl FnOnce(&mut Config)) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let Ok(mut cfg) = toml::from_str::<Config>(&content) else { return };
    f(&mut cfg);
    if let Ok(text) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, text);
    }
}

pub fn save_distill_mode_to_config(mode_name: &str) {
    reserialize("config.toml", |cfg| cfg.distill_mode = Some(mode_name.to_string()));
}

pub fn save_modes_to_config(modes: &[ModeConfig]) {
    reserialize("config.toml", |cfg| cfg.modes = modes.to_vec());
}

pub const DEFAULT_SETTINGS_W: u32 = 480;
pub const DEFAULT_SETTINGS_H: u32 = 290;

pub fn load_settings_size() -> (u32, u32) {
    let raw = std::fs::read_to_string("config.toml").unwrap_or_default();
    let Ok(cfg) = toml::from_str::<Config>(&raw) else {
        return (DEFAULT_SETTINGS_W, DEFAULT_SETTINGS_H);
    };
    (
        cfg.settings_w.unwrap_or(DEFAULT_SETTINGS_W),
        cfg.settings_h.unwrap_or(DEFAULT_SETTINGS_H),
    )
}

pub fn save_settings_size_to_config(w: u32, h: u32) {
    reserialize("config.toml", |cfg| {
        cfg.settings_w = Some(w);
        cfg.settings_h = Some(h);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_TOML: &str = r#"
        whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
        ollama_url    = "http://localhost:11434/api/generate"
        whisper_model = "whisper-large-turbo"
        llm_model     = "qwen2.5:7b-instruct"
        prompt        = "Fix grammar.\n\n---\n"
    "#;

    #[test]
    fn valid_toml_parses() {
        let cfg = parse_config(BASE_TOML).unwrap();
        assert_eq!(cfg.llm_model, "qwen2.5:7b-instruct");
        assert_eq!(cfg.whisper_model, "whisper-large-turbo");
        assert_eq!(cfg.whisper_url, "http://localhost:8080/v1/audio/transcriptions");
        assert_eq!(cfg.ollama_url, "http://localhost:11434/api/generate");
        assert_eq!(cfg.hotkey, None);
    }

    #[test]
    fn hotkey_field_parses() {
        let raw = format!("{BASE_TOML}\nhotkey = \"Alt+W\"");
        let cfg = parse_config(&raw).unwrap();
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
    fn modes_parse_from_toml() {
        let raw = format!(r#"
            {BASE_TOML}
            distill_mode = "Translate to English"

            [[modes]]
            name   = "Distill"
            prompt = "Fix grammar.\n"

            [[modes]]
            name   = "Translate to English"
            prompt = "Translate.\n"
        "#);
        let cfg = parse_config(&raw).unwrap();
        assert_eq!(cfg.modes.len(), 2);
        assert_eq!(cfg.modes[0].name, "Distill");
        assert_eq!(cfg.modes[1].name, "Translate to English");
        assert_eq!(cfg.distill_mode.as_deref(), Some("Translate to English"));
    }

    #[test]
    fn default_modes_uses_legacy_prompt() {
        let modes = default_modes("my-prompt");
        assert_eq!(modes[0].name, "Distill");
        assert_eq!(modes[0].prompt, "my-prompt");
        assert!(modes.len() >= 3);
    }

    #[test]
    fn settings_size_fields_parse() {
        let raw = format!("{BASE_TOML}\nsettings_w = 500\nsettings_h = 350");
        let cfg = parse_config(&raw).unwrap();
        assert_eq!(cfg.settings_w, Some(500u32));
        assert_eq!(cfg.settings_h, Some(350u32));
    }

}
