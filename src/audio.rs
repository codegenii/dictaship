use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use enigo::{Enigo, Keyboard, Settings};
use hound::{WavSpec, WavWriter};
use parking_lot::Mutex;
use std::{io::Cursor, sync::Arc, time::Duration};

use crate::config::Config;

#[derive(serde::Deserialize)]
struct WhisperResp { text: String }
#[derive(serde::Deserialize)]
struct OllamaResp  { response: String }

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

pub fn is_too_short(samples: &[i16], sample_rate: u32) -> bool {
    samples.len() < sample_rate as usize / 2
}

pub fn process(
    samples: Vec<i16>,
    sample_rate: u32,
    cfg: Arc<Config>,
    status: Arc<Mutex<Option<String>>>,
    passthrough: bool,
) {
    let set = |s: Option<&str>| *status.lock() = s.map(str::to_owned);

    if is_too_short(&samples, sample_rate) {
        eprintln!("error: recording too short");
        set(None);
        return;
    }
    let wav = match samples_to_wav(&samples, sample_rate) {
        Ok(w) => w,
        Err(e) => { eprintln!("error: {e:#}"); set(None); return; }
    };
    println!("transcribing {} samples...", samples.len());
    let transcript = match transcribe(wav, &cfg) {
        Ok(t) => t,
        Err(e) => { eprintln!("error: {e:#}"); set(None); return; }
    };
    println!("raw: {transcript}");

    if passthrough {
        println!("out (passthrough): {transcript}");
        if let Err(e) = paste(&transcript) { eprintln!("error: {e:#}"); }
        set(None);
        return;
    }

    set(Some("Distilling..."));
    let distilled = match distill(&transcript, &cfg) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e:#}"); set(None); return; }
    };
    println!("out: {distilled}");
    if let Err(e) = paste(&distilled) { eprintln!("error: {e:#}"); }
    set(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    mod wav {
        use super::*;

        const SAMPLE_RATE: u32 = 16_000;

        fn header_u16(wav: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes([wav[offset], wav[offset + 1]])
        }

        fn header_u32(wav: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes([
                wav[offset], wav[offset + 1], wav[offset + 2], wav[offset + 3],
            ])
        }

        #[test]
        fn magic_bytes_are_correct() {
            let wav = samples_to_wav(&[0i16; 100], SAMPLE_RATE).unwrap();
            assert_eq!(&wav[0..4],   b"RIFF");
            assert_eq!(&wav[8..12],  b"WAVE");
            assert_eq!(&wav[12..16], b"fmt ");
            assert_eq!(&wav[36..40], b"data");
        }

        #[test]
        fn header_encodes_correct_format() {
            let wav = samples_to_wav(&[0i16; 100], SAMPLE_RATE).unwrap();
            assert_eq!(header_u16(&wav, 20), 1);           // PCM format
            assert_eq!(header_u16(&wav, 22), 1);           // mono
            assert_eq!(header_u32(&wav, 24), SAMPLE_RATE);
            assert_eq!(header_u16(&wav, 34), 16);          // 16-bit
        }

        #[test]
        fn header_reflects_device_sample_rate() {
            let wav = samples_to_wav(&[0i16; 100], 48_000).unwrap();
            assert_eq!(header_u32(&wav, 24), 48_000);
        }

        #[test]
        fn empty_samples_is_valid() {
            let wav = samples_to_wav(&[], SAMPLE_RATE).unwrap();
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(header_u32(&wav, 40), 0);
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
    }
}
