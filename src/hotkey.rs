use global_hotkey::hotkey::{Code, HotKey, Modifiers};

pub fn parse_hotkey(s: &str) -> Option<HotKey> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (mod_parts, key_parts) = parts.split_at(parts.len().saturating_sub(1));
    let key_str = key_parts.first()?;

    let code = match key_str.to_uppercase().as_str() {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "F1"  => Code::F1,  "F2"  => Code::F2,  "F3"  => Code::F3,  "F4"  => Code::F4,
        "F5"  => Code::F5,  "F6"  => Code::F6,  "F7"  => Code::F7,  "F8"  => Code::F8,
        "F9"  => Code::F9,  "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        _ => return None,
    };

    let mut mods = Modifiers::empty();
    for m in mod_parts {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt"              => mods |= Modifiers::ALT,
            "shift"            => mods |= Modifiers::SHIFT,
            "meta" | "win"     => mods |= Modifiers::META,
            _                  => return None,
        }
    }

    Some(HotKey::new(if mods.is_empty() { None } else { Some(mods) }, code))
}

fn apply_hotkey(content: &str, hotkey_str: &str) -> String {
    let line = format!("hotkey = \"{}\"", hotkey_str);
    if content.lines().any(|l| l.trim_start().starts_with("hotkey")) {
        content.lines()
            .map(|l| if l.trim_start().starts_with("hotkey") { line.as_str() } else { l })
            .collect::<Vec<_>>()
            .join("\r\n")
    } else {
        format!("{}\n{}\n", content.trim_end(), line)
    }
}

pub fn save_hotkey_to_config(hotkey_str: &str) {
    let path = "config.toml";
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let _ = std::fs::write(path, apply_hotkey(&content, hotkey_str));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_q_parses() {
        assert!(parse_hotkey("Alt+Q").is_some());
    }

    #[test]
    fn ctrl_alt_r_parses() {
        assert!(parse_hotkey("Ctrl+Alt+R").is_some());
    }

    #[test]
    fn function_keys_parse() {
        assert!(parse_hotkey("F9").is_some());
        assert!(parse_hotkey("F12").is_some());
    }

    #[test]
    fn unknown_key_fails() {
        assert!(parse_hotkey("Alt+7").is_none());
        assert!(parse_hotkey("Foo+Q").is_none());
        assert!(parse_hotkey("").is_none());
    }

    #[test]
    fn default_hotkey_is_valid() {
        assert!(parse_hotkey(crate::settings_dialog::DEFAULT_HOTKEY).is_some());
    }

    #[test]
    fn hotkey_parsing_is_case_insensitive() {
        let lower = parse_hotkey("ctrl+alt+r").unwrap();
        let mixed = parse_hotkey("Ctrl+Alt+R").unwrap();
        assert_eq!(lower.id(), mixed.id());
    }

    #[test]
    fn win_modifier_parses() {
        assert!(parse_hotkey("Win+R").is_some());
        assert!(parse_hotkey("Meta+R").is_some());
        // Both spellings produce the same hotkey
        assert_eq!(
            parse_hotkey("Win+R").unwrap().id(),
            parse_hotkey("Meta+R").unwrap().id(),
        );
    }

    #[test]
    fn bare_key_without_modifier_parses() {
        assert!(parse_hotkey("Q").is_some());
        assert!(parse_hotkey("F5").is_some());
    }

    #[test]
    fn save_hotkey_replaces_existing_line() {
        let content = "whisper_url = \"x\"\nhotkey = \"Alt+R\"\nllm_model = \"y\"";
        let result = apply_hotkey(content, "Alt+W");
        assert!(result.contains("hotkey = \"Alt+W\""), "new key present");
        assert!(!result.contains("Alt+R"),             "old key removed");
        assert!(result.contains("whisper_url"),        "other fields preserved");
        assert!(result.contains("llm_model"),          "other fields preserved");
        let hotkey_lines = result.lines().filter(|l| l.trim_start().starts_with("hotkey")).count();
        assert_eq!(hotkey_lines, 1, "exactly one hotkey line");
    }

    #[test]
    fn save_hotkey_appends_when_missing() {
        let content = "whisper_url = \"x\"\nllm_model = \"y\"";
        let result = apply_hotkey(content, "Alt+W");
        assert!(result.contains("hotkey = \"Alt+W\""), "hotkey appended");
        assert!(result.contains("whisper_url"),        "other fields preserved");
        assert!(result.contains("llm_model"),          "other fields preserved");
        let hotkey_lines = result.lines().filter(|l| l.trim_start().starts_with("hotkey")).count();
        assert_eq!(hotkey_lines, 1, "exactly one hotkey line");
    }
}
