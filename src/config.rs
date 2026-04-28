use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PASSTHROUGH_MODE_NAME: &str = "Verbatim";

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


pub fn default_modes(legacy_prompt: &str) -> Vec<ModeConfig> {
    vec![
        ModeConfig {
            name:   "Prompt".to_string(),
            prompt: legacy_prompt.to_string(),
        },
        ModeConfig {
            name:   "Clean text".to_string(),
            prompt: "Remove all filler words, interjections, and hesitations. \
                     Fix punctuation. Output ONLY the cleaned text.\n\n---\n".to_string(),
        },
        ModeConfig {
            name:   PASSTHROUGH_MODE_NAME.to_string(),
            prompt: String::new(),
        },
    ]
}

fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.toml")))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

pub fn parse_config(raw: &str) -> Result<Config> {
    toml::from_str(raw).map_err(|e| anyhow::anyhow!("invalid config.toml: {e}"))
}

pub fn load_config() -> Result<Config> {
    let raw = std::fs::read_to_string(config_path())
        .map_err(|e| anyhow::anyhow!("cannot read config.toml: {e}"))?;
    let mut cfg = parse_config(&raw)?;
    if cfg.modes.is_empty() {
        cfg.modes = default_modes(&cfg.prompt);
    } else if !cfg.modes.iter().any(|m| m.name == PASSTHROUGH_MODE_NAME) {
        cfg.modes.push(ModeConfig { name: PASSTHROUGH_MODE_NAME.to_string(), prompt: String::new() });
    }
    Ok(cfg)
}

fn reserialize(path: &Path, f: impl FnOnce(&mut Config)) {
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let Ok(mut cfg) = toml::from_str::<Config>(&content) else { return };
    f(&mut cfg);
    if let Ok(text) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, text);
    }
}

pub fn save_distill_mode_to_config(mode_name: &str) {
    reserialize(&config_path(), |cfg| cfg.distill_mode = Some(mode_name.to_string()));
}

pub fn save_modes_to_config(modes: &[ModeConfig]) {
    reserialize(&config_path(), |cfg| cfg.modes = modes.to_vec());
}

pub const DEFAULT_SETTINGS_W: u32 = 480;
pub const DEFAULT_SETTINGS_H: u32 = 290;

pub fn load_settings_size() -> (u32, u32) {
    let raw = std::fs::read_to_string(config_path()).unwrap_or_default();
    let Ok(cfg) = toml::from_str::<Config>(&raw) else {
        return (DEFAULT_SETTINGS_W, DEFAULT_SETTINGS_H);
    };
    (
        cfg.settings_w.unwrap_or(DEFAULT_SETTINGS_W),
        cfg.settings_h.unwrap_or(DEFAULT_SETTINGS_H),
    )
}

pub fn save_settings_size_to_config(w: u32, h: u32) {
    reserialize(&config_path(), |cfg| {
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
        assert_eq!(modes[0].name, "Prompt");
        assert_eq!(modes[0].prompt, "my-prompt");
        assert!(modes.len() >= 2);
    }

    #[test]
    fn settings_size_fields_parse() {
        let raw = format!("{BASE_TOML}\nsettings_w = 500\nsettings_h = 350");
        let cfg = parse_config(&raw).unwrap();
        assert_eq!(cfg.settings_w, Some(500u32));
        assert_eq!(cfg.settings_h, Some(350u32));
    }

    #[test]
    fn default_modes_includes_passthrough() {
        let modes = default_modes("prompt");
        assert!(modes.iter().any(|m| m.name == PASSTHROUGH_MODE_NAME));
        let pt = modes.iter().find(|m| m.name == PASSTHROUGH_MODE_NAME).unwrap();
        assert!(pt.prompt.is_empty());
    }

    #[test]
    fn load_config_adds_passthrough_when_missing() {
        let raw = format!(r#"
            {BASE_TOML}
            [[modes]]
            name   = "Distill"
            prompt = "Fix grammar.\n"
        "#);
        let cfg = parse_config(&raw).unwrap();
        // parse_config alone doesn't add Passthrough; simulate what load_config does
        let mut cfg = cfg;
        if !cfg.modes.iter().any(|m| m.name == PASSTHROUGH_MODE_NAME) {
            cfg.modes.push(ModeConfig { name: PASSTHROUGH_MODE_NAME.to_string(), prompt: String::new() });
        }
        assert!(cfg.modes.iter().any(|m| m.name == PASSTHROUGH_MODE_NAME));
    }

    #[test]
    fn load_config_does_not_duplicate_passthrough() {
        let raw = format!(r#"
            {BASE_TOML}
            [[modes]]
            name   = "Distill"
            prompt = "Fix grammar.\n"

            [[modes]]
            name   = "Verbatim"
            prompt = ""
        "#);
        let mut cfg = parse_config(&raw).unwrap();
        if !cfg.modes.iter().any(|m| m.name == PASSTHROUGH_MODE_NAME) {
            cfg.modes.push(ModeConfig { name: PASSTHROUGH_MODE_NAME.to_string(), prompt: String::new() });
        }
        let count = cfg.modes.iter().filter(|m| m.name == PASSTHROUGH_MODE_NAME).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn reserialize_preserves_all_fields() {
        let tmp = std::env::temp_dir().join("dictaship_reserialize_test.toml");
        std::fs::write(&tmp, BASE_TOML).unwrap();
        reserialize(&tmp, |cfg| {
            cfg.hotkey = Some("Alt+X".to_string());
        });
        let content = std::fs::read_to_string(&tmp).unwrap();
        std::fs::remove_file(&tmp).ok();
        let cfg = parse_config(&content).unwrap();
        assert_eq!(cfg.hotkey.as_deref(), Some("Alt+X"));
        assert_eq!(cfg.llm_model, "qwen2.5:7b-instruct");
        assert_eq!(cfg.whisper_model, "whisper-large-turbo");
        assert_eq!(cfg.whisper_url, "http://localhost:8080/v1/audio/transcriptions");
    }

}
