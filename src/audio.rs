use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use enigo::{Enigo, Keyboard, Settings};
use hound::{WavSpec, WavWriter};
use parking_lot::Mutex;
use std::{io::Cursor, sync::Arc, thread, time::Duration};

use crate::config::Config;

#[derive(serde::Deserialize)]
struct WhisperResp { text: String }
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum OllamaResp {
    Ok    { response: String },
    Error { error: String },
}

pub struct Recorder {
    samples:     Arc<Mutex<Vec<i16>>>,
    stream:      Option<cpal::Stream>,
    sample_rate: u32,
}

impl Recorder {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no mic"))?;
        let supported = device.default_input_config()?;
        let channels = supported.channels();
        let sample_rate = supported.sample_rate().0;
        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };
        let samples = Arc::new(Mutex::new(
            Vec::<i16>::with_capacity(sample_rate as usize * 30),
        ));
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

    pub fn stop(mut self) -> (Vec<i16>, u32) {
        drop(self.stream.take());
        let samples = std::mem::take(&mut *self.samples.lock());
        (samples, self.sample_rate)
    }
}

pub fn samples_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
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

fn distill(text: &str, cfg: &Config, prompt: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120)).build()?;
    let body = serde_json::json!({
        "model": cfg.llm_model,
        "prompt": format!("{prompt}{text}"),
        "stream": false,
    });
    match client.post(&cfg.ollama_url).json(&body).send()?.json()? {
        OllamaResp::Ok    { response } => Ok(response.trim().to_string()),
        OllamaResp::Error { error }    => Err(anyhow::anyhow!("llm: {error}")),
    }
}

fn paste(text: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.text(text)?;
    Ok(())
}

pub fn is_too_short(samples: &[i16], sample_rate: u32) -> bool {
    samples.len() < sample_rate as usize / 2
}

/// Show `label` in the tray balloon/tooltip and log `detail` to the console.
/// Auto-clears the status after 6 seconds so the tray returns to idle.
pub fn set_error_status(
    status: &Arc<Mutex<Option<String>>>,
    label: &'static str,
    detail: impl std::fmt::Display,
) {
    eprintln!("error: {detail:#}");
    *status.lock() = Some(label.to_owned());
    let status = status.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(6));
        let mut s = status.lock();
        if s.as_deref() == Some(label) {
            *s = None;
        }
    });
}

pub fn process(
    samples: Vec<i16>,
    sample_rate: u32,
    cfg: Arc<Config>,
    status: Arc<Mutex<Option<String>>>,
    passthrough: bool,
    active_prompt: String,
) {
    let set = |s: Option<&str>| *status.lock() = s.map(str::to_owned);
    let err = |label, detail| set_error_status(&status, label, detail);

    if is_too_short(&samples, sample_rate) {
        err("Recording too short", "recording too short".to_string());
        return;
    }
    let wav = match samples_to_wav(&samples, sample_rate) {
        Ok(w) => w,
        Err(e) => { err("Recording error", format!("{e:#}")); return; }
    };
    println!("transcribing {} samples...", samples.len());
    let transcript = match transcribe(wav, &cfg) {
        Ok(t) => t,
        Err(e) => { err("Transcription failed", format!("{e:#}")); return; }
    };
    println!("raw: {transcript}");

    if passthrough {
        println!("out (passthrough): {transcript}");
        match paste(&transcript) {
            Ok(()) => set(None),
            Err(e)  => err("Paste failed", format!("{e:#}")),
        }
        return;
    }

    set(Some("Distilling..."));
    let distilled = match distill(&transcript, &cfg, &active_prompt) {
        Ok(d) => d,
        Err(e) => { err("Distillation failed", format!("{e:#}")); return; }
    };
    println!("out: {distilled}");
    match paste(&distilled) {
        Ok(()) => set(None),
        Err(e)  => err("Paste failed", format!("{e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod wav {
        use super::*;

        fn header_u32(wav: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes(wav[offset..offset + 4].try_into().unwrap())
        }

        const SAMPLE_RATE: u32 = 16_000;

        #[test]
        fn empty_samples_is_valid() {
            let wav = samples_to_wav(&[], SAMPLE_RATE).unwrap();
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(header_u32(&wav, 40), 0);
        }

        #[test]
        fn single_sample_has_correct_data_size() {
            let wav = samples_to_wav(&[1000i16], SAMPLE_RATE).unwrap();
            assert_eq!(header_u32(&wav, 40), 2); // 1 sample × 2 bytes (i16)
        }

        #[test]
        fn samples_round_trip() {
            let original: Vec<i16> = (0..64).map(|i| (i * 100) as i16).collect();
            let wav = samples_to_wav(&original, SAMPLE_RATE).unwrap();
            let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
            let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
            assert_eq!(decoded, original);
        }
    }

    mod errors {
        use super::*;

        #[test]
        fn set_error_status_sets_tray_label() {
            let status: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            set_error_status(&status, "Test error", "some detail");
            assert_eq!(status.lock().as_deref(), Some("Test error"));
        }

    }

    mod recording {
        use super::*;

        const SAMPLE_RATE: u32 = 16_000;

        #[test]
        fn too_short_is_detected() {
            assert!(is_too_short(&vec![0i16; SAMPLE_RATE as usize / 2 - 1], SAMPLE_RATE));
        }

        #[test]
        fn at_threshold_is_accepted() {
            assert!(!is_too_short(&vec![0i16; SAMPLE_RATE as usize / 2], SAMPLE_RATE));
        }

        #[test]
        fn threshold_scales_with_sample_rate() {
            let rate = 48_000u32;
            assert!(is_too_short(&vec![0i16; rate as usize / 2 - 1], rate));
            assert!(!is_too_short(&vec![0i16; rate as usize / 2], rate));
        }

        #[test]
        fn zero_samples_is_too_short() {
            assert!(is_too_short(&[], SAMPLE_RATE));
        }
    }
}
