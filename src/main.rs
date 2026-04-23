use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use enigo::{Enigo, Keyboard, Settings};
use global_hotkey::{hotkey::{Code, HotKey, Modifiers}, GlobalHotKeyManager, GlobalHotKeyEvent};
use hound::{WavSpec, WavWriter};
use muda::{Menu, MenuItem, MenuEvent};
use parking_lot::Mutex;
use serde::Deserialize;
use std::{io::Cursor, sync::Arc, thread, time::Duration};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

mod console_window {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> isize;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, n_cmd_show: i32) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
    }

    pub fn hide() {
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd != 0 { ShowWindow(hwnd, 0); }
        }
    }

    pub fn toggle() {
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd == 0 { return; }
            if IsWindowVisible(hwnd) != 0 {
                ShowWindow(hwnd, 0); // SW_HIDE
            } else {
                ShowWindow(hwnd, 5); // SW_SHOW
            }
        }
    }
}

fn make_icon() -> tray_icon::Icon {
    const S: u32 = 32;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    let c = S as f32 / 2.0;
    let r = S as f32 / 2.0 - 1.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            if dx * dx + dy * dy <= r * r {
                rgba.extend_from_slice(&[34, 197, 94, 255]); // green circle
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, S, S).expect("valid icon")
}

#[derive(Deserialize)]
struct Config {
    whisper_url:   String,
    ollama_url:    String,
    whisper_model: String,
    llm_model:     String,
    prompt:        String,
}

fn parse_config(raw: &str) -> Result<Config> {
    toml::from_str(raw).map_err(|e| anyhow::anyhow!("invalid config.toml: {e}"))
}

fn load_config() -> Result<Config> {
    let raw = std::fs::read_to_string("config.toml")
        .map_err(|e| anyhow::anyhow!("cannot read config.toml: {e}"))?;
    parse_config(&raw)
}

#[derive(Deserialize)]
struct WhisperResp { text: String }
#[derive(Deserialize)]
struct OllamaResp  { response: String }

struct Recorder {
    samples: Arc<Mutex<Vec<i16>>>,
    stream: Option<cpal::Stream>,
    sample_rate: u32,
}

impl Recorder {
    fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| anyhow::anyhow!("no mic"))?;
        let supported = device.default_input_config()?;
        let channels = supported.channels();
        let sample_rate = supported.sample_rate().0;
        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };
        let samples = Arc::new(Mutex::new(Vec::<i16>::with_capacity(sample_rate as usize * 30)));
        let samples_cb = samples.clone();
        let stream = device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &_| {
                let mut buf = samples_cb.lock();
                buf.extend(data.chunks(channels as usize).map(|frame| {
                    let mono = frame.iter().sum::<f32>() / channels as f32;
                    (mono * i16::MAX as f32) as i16
                }));
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )?;
        stream.play()?;
        Ok(Self { samples, stream: Some(stream), sample_rate })
    }

    fn stop(mut self) -> (Vec<i16>, u32) {
        drop(self.stream.take());
        let samples = std::mem::take(&mut *self.samples.lock());
        (samples, self.sample_rate)
    }
}

fn samples_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1, sample_rate,
        bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::<u8>::new());
    {
        let mut w = WavWriter::new(&mut buf, spec)?;
        for &s in samples { w.write_sample(s)?; }
        w.finalize()?;
    }
    Ok(buf.into_inner())
}

fn transcribe(wav: Vec<u8>, cfg: &Config) -> Result<String> {
    let client = reqwest::blocking::Client::new();
    let part = reqwest::blocking::multipart::Part::bytes(wav)
        .file_name("audio.wav").mime_str("audio/wav")?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("model", cfg.whisper_model.clone())
        .part("file", part);
    let resp: WhisperResp = client.post(&cfg.whisper_url).multipart(form).send()?.json()?;
    Ok(resp.text.trim().to_string())
}

fn distill(text: &str, cfg: &Config) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120)).build()?;
    let body = serde_json::json!({
        "model": cfg.llm_model,
        "prompt": format!("{}{text}", cfg.prompt),
        "stream": false,
    });
    let resp: OllamaResp = client.post(&cfg.ollama_url).json(&body).send()?.json()?;
    Ok(resp.response.trim().to_string())
}

fn paste(text: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.text(text)?;
    Ok(())
}

fn is_too_short(samples: &[i16], sample_rate: u32) -> bool {
    samples.len() < sample_rate as usize / 2
}

fn process(samples: Vec<i16>, sample_rate: u32, cfg: Arc<Config>) {
    let result = (|| -> Result<()> {
        if is_too_short(&samples, sample_rate) {
            anyhow::bail!("too short");
        }
        let wav = samples_to_wav(&samples, sample_rate)?;
        println!("transcribing {} samples...", samples.len());
        let transcript = transcribe(wav, &cfg)?;
        println!("raw: {transcript}");
        let distilled = distill(&transcript, &cfg)?;
        println!("out: {distilled}");
        paste(&distilled)?;
        Ok(())
    })();
    if let Err(e) = result { eprintln!("error: {e:#}"); }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 16_000;

    const VALID_TOML: &str = r#"
        whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
        ollama_url    = "http://localhost:11434/api/generate"
        whisper_model = "whisper-large-turbo"
        llm_model     = "qwen2.5:7b-instruct"
        prompt        = "Fix grammar.\n\n---\n"
    "#;

    // --- config parsing ---

    #[test]
    fn config_valid_toml_parses() {
        let cfg = parse_config(VALID_TOML).unwrap();
        assert_eq!(cfg.llm_model, "qwen2.5:7b-instruct");
        assert_eq!(cfg.whisper_model, "whisper-large-turbo");
        assert_eq!(cfg.whisper_url, "http://localhost:8080/v1/audio/transcriptions");
        assert_eq!(cfg.ollama_url, "http://localhost:11434/api/generate");
        assert_eq!(cfg.prompt, "Fix grammar.\n\n---\n");
    }

    #[test]
    fn config_missing_field_fails() {
        let raw = r#"
            whisper_url = "http://localhost:8080"
            ollama_url  = "http://localhost:11434"
        "#;
        assert!(parse_config(raw).is_err());
    }

    #[test]
    fn config_invalid_toml_fails() {
        assert!(parse_config("this is not toml :::").is_err());
    }

    // --- WAV encoding ---

    fn wav_header_u16(wav: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([wav[offset], wav[offset + 1]])
    }

    fn wav_header_u32(wav: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([wav[offset], wav[offset + 1], wav[offset + 2], wav[offset + 3]])
    }

    #[test]
    fn wav_magic_bytes_are_correct() {
        let wav = samples_to_wav(&[0i16; 100], SAMPLE_RATE).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
    }

    #[test]
    fn wav_header_encodes_correct_format() {
        let wav = samples_to_wav(&[0i16; 100], SAMPLE_RATE).unwrap();
        assert_eq!(wav_header_u16(&wav, 20), 1);          // PCM format
        assert_eq!(wav_header_u16(&wav, 22), 1);          // mono
        assert_eq!(wav_header_u32(&wav, 24), SAMPLE_RATE); // sample rate
        assert_eq!(wav_header_u16(&wav, 34), 16);         // bits per sample
    }

    #[test]
    fn wav_header_reflects_device_sample_rate() {
        let wav = samples_to_wav(&[0i16; 100], 48_000).unwrap();
        assert_eq!(wav_header_u32(&wav, 24), 48_000);
    }

    #[test]
    fn wav_empty_samples_is_valid() {
        let wav = samples_to_wav(&[], SAMPLE_RATE).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav_header_u32(&wav, 40), 0); // data chunk has 0 bytes
    }

    #[test]
    fn wav_samples_round_trip() {
        let original: Vec<i16> = (0..64).map(|i| (i * 100) as i16).collect();
        let wav = samples_to_wav(&original, SAMPLE_RATE).unwrap();
        let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded, original);
    }

    // --- minimum-length guard ---

    #[test]
    fn recording_too_short_is_detected() {
        assert!(is_too_short(&vec![0i16; SAMPLE_RATE as usize / 2 - 1], SAMPLE_RATE));
    }

    #[test]
    fn recording_at_threshold_is_accepted() {
        assert!(!is_too_short(&vec![0i16; SAMPLE_RATE as usize / 2], SAMPLE_RATE));
    }

    #[test]
    fn too_short_threshold_scales_with_sample_rate() {
        let rate = 48_000u32;
        assert!(is_too_short(&vec![0i16; rate as usize / 2 - 1], rate));
        assert!(!is_too_short(&vec![0i16; rate as usize / 2], rate));
    }
}

fn main() -> Result<()> {
    let cfg = Arc::new(load_config()?);

    console_window::hide();

    let event_loop = EventLoopBuilder::new().build();
    let manager = GlobalHotKeyManager::new()?;
    let toggle = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR);
    manager.register(toggle)?;
    let rx = GlobalHotKeyEvent::receiver();

    let tray_menu = Menu::new();
    let exit_item = MenuItem::new("Exit", true, None);
    tray_menu.append(&exit_item).expect("menu append");

    let _tray = TrayIconBuilder::new()
        .with_icon(make_icon())
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Partizan – Ctrl+Alt+R to record")
        .build()
        .expect("tray icon");

    let menu_rx = MenuEvent::receiver();
    let tray_rx = TrayIconEvent::receiver();

    let mut recorder: Option<Recorder> = None;
    println!("ready. Ctrl+Alt+R to toggle recording.");

    event_loop.run(move |_, _, cf| {
        *cf = ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_millis(50));

        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == *exit_item.id() {
                std::process::exit(0);
            }
        }

        while let Ok(ev) = tray_rx.try_recv() {
            if let TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. } = ev {
                console_window::toggle();
            }
        }

        while let Ok(ev) = rx.try_recv() {
            if ev.id == toggle.id() && ev.state == global_hotkey::HotKeyState::Pressed {
                match recorder.take() {
                    None => match Recorder::start() {
                        Ok(r) => { recorder = Some(r); println!("recording..."); }
                        Err(e) => eprintln!("mic error: {e}"),
                    },
                    Some(r) => {
                        println!("stopping.");
                        let (samples, sample_rate) = r.stop();
                        let cfg = cfg.clone();
                        thread::spawn(move || process(samples, sample_rate, cfg));
                    }
                }
            }
        }
    });
}
