//! Open-mic speech recognition: microphone -> Silero VAD (here) -> a speech-to-text server.

use crate::config::{path_str, Config};
use crate::link::GameLink;
use serde_json::{json, Value};
use anyhow::{anyhow, bail, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use sherpa_onnx::{LinearResampler, VadModelConfig, VoiceActivityDetector};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Instant;

use crate::Speaking;

const SAMPLE_RATE: i32 = 16000;
/// Silero works on fixed 512-sample windows at 16 kHz.
const VAD_WINDOW: usize = 512;
/// Mic audio kept around each piece of speech the VAD finds, before and after it: its cut
/// clips a short word's start and end, which the recognizer then can't make out.
const MARGIN_BEFORE: usize = SAMPLE_RATE as usize / 5;
const MARGIN_AFTER: usize = SAMPLE_RATE as usize / 10;
/// The mic audio kept for that (more than the longest piece, 20 s).
const KEPT: usize = SAMPLE_RATE as usize * 25;

/// Speech to text, from a server: an OpenAI-style /v1/audio/transcriptions (a WAV in, its text
/// out). For another kind of service, `transcribe` is the one place to change.
#[derive(Clone)]
pub struct Transcriber {
    url: String,
    model: Option<String>,
    agent: ureq::Agent,
}

impl Transcriber {
    pub fn new(cfg: &Config) -> Self {
        let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(30)).build();
        Transcriber { url: cfg.stt.url.clone(), model: cfg.stt.model.clone(), agent }
    }

    /// What's said in 16 kHz mono audio.
    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        const BOUNDARY: &str = "m8-audio-boundary";
        let mut body = Vec::new();
        if let Some(model) = &self.model {
            body.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{model}\r\n").bytes());
        }
        body.extend(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n")
                .bytes(),
        );
        body.extend(wav(samples, SAMPLE_RATE as u32));
        body.extend(format!("\r\n--{BOUNDARY}--\r\n").bytes());
        let reply: Value = self
            .agent
            .post(&self.url)
            .set("Content-Type", &format!("multipart/form-data; boundary={BOUNDARY}"))
            .send_bytes(&body)
            .map_err(|e| anyhow!("speech-to-text at {}: {e}", self.url))?
            .into_json()?;
        Ok(reply["text"].as_str().unwrap_or_default().trim().to_string())
    }
}

/// A WAV of mono audio (-1 to 1), 16-bit.
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for s in samples {
        out.extend(((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn parakeet_multipart_contains_model_and_mono_16k_pcm16_wav() {
        let server = MockHttp::new(|_, stream| {
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                br#"{"text":"  Cover me!  "}"#,
            )
        });
        let mut cfg = config();
        cfg.stt.url = format!("{}/v1/audio/transcriptions", server.url);
        let samples = [-1.0, 0.0, 1.0];
        assert_eq!(
            Transcriber::new(&cfg).transcribe(&samples).unwrap(),
            "Cover me!"
        );
        let request = server.request();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/audio/transcriptions");
        assert!(request.headers["content-type"].starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"model\"\r\n\r\nparakeet\r\n"));
        assert!(body.contains("name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav"));
        assert!(!body.contains("diarization"));
        let start = request.body.windows(4).position(|p| p == b"RIFF").unwrap();
        let wave = &request.body[start..start + 50];
        assert_eq!(&wave[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(wave[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(wave[22..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(wave[24..28].try_into().unwrap()), 16000);
        assert_eq!(u16::from_le_bytes(wave[34..36].try_into().unwrap()), 16);
        assert_eq!(&wave[44..], &[1, 128, 0, 0, 255, 127]);
    }
}

fn create_vad(cfg: &Config) -> Result<VoiceActivityDetector> {
    let mut vc = VadModelConfig::default();
    vc.silero_vad.model = Some(path_str(&cfg.models.silero_vad));
    vc.silero_vad.threshold = cfg.mic.vad_threshold;
    vc.silero_vad.min_silence_duration = cfg.mic.end_of_speech_silence;
    // (none: a quick word is speech too)
    vc.silero_vad.min_speech_duration = 0.0;
    vc.silero_vad.max_speech_duration = 20.0;
    vc.silero_vad.window_size = VAD_WINDOW as i32;
    vc.sample_rate = SAMPLE_RATE;
    VoiceActivityDetector::create(&vc, 60.0).ok_or_else(|| anyhow!("failed to load Silero VAD"))
}

/// Turns raw audio at the input's sample rate into finished utterances.
pub struct Listener {
    cfg: Config,
    transcriber: Transcriber,
    vad: VoiceActivityDetector,
    resampler: Option<LinearResampler>,
    pending: Vec<f32>,
    was_muted: bool,
    /// How much loud audio arrived while muted, to tell the user they were talked over.
    loud_while_muted: usize,
    recordings: Option<PathBuf>,
    segment_index: usize,
    /// The mic audio the VAD has had lately, from sample `kept_from` on (counting every sample
    /// it has had), and where its own count started (it starts again when reset).
    kept: std::collections::VecDeque<f32>,
    kept_from: usize,
    accepted: usize,
    vad_origin: usize,
}

impl Listener {
    pub fn new(cfg: Config, transcriber: Transcriber, input_rate: i32) -> Result<Self> {
        let recordings = cfg.mic.save_recordings.then(|| cfg.base_dir.join("recordings"));
        if let Some(dir) = &recordings {
            std::fs::create_dir_all(dir)?;
            println!("[mic] saving what it hears to {}", dir.display());
        }
        Ok(Listener {
            vad: create_vad(&cfg)?,
            resampler: (input_rate != SAMPLE_RATE)
                .then(|| LinearResampler::create(input_rate, SAMPLE_RATE))
                .flatten(),
            cfg,
            transcriber,
            pending: Vec::new(),
            was_muted: false,
            loud_while_muted: 0,
            recordings,
            segment_index: 0,
            kept: std::collections::VecDeque::new(),
            kept_from: 0,
            accepted: 0,
            vad_origin: 0,
        })
    }

    /// Feeds one chunk of input audio; returns the text of any utterances it completed.
    pub fn feed(&mut self, chunk: Vec<f32>, muted: bool) -> Vec<String> {
        let samples = match &self.resampler {
            Some(r) => r.resample(&chunk, false),
            None => chunk,
        };

        // Half duplex: drop mic audio while the mate is talking (plus a short tail).
        if muted {
            if !self.was_muted {
                self.vad.reset();
                self.vad_origin = self.accepted;
                self.pending.clear();
                self.loud_while_muted = 0;
            }
            self.was_muted = true;
            if level_db(&samples).0 > -40.0 {
                self.loud_while_muted += samples.len();
            }
            return Vec::new();
        }
        if self.was_muted && self.loud_while_muted > SAMPLE_RATE as usize * 3 / 10 {
            println!(
                "[mic] ignored ~{:.1}s of sound while {} was talking (with headphones, set mute_while_speaking = false)",
                self.loud_while_muted as f32 / SAMPLE_RATE as f32,
                self.cfg.mate.name
            );
        }
        self.was_muted = false;

        self.pending.extend_from_slice(&samples);
        let mut offset = 0;
        while offset + VAD_WINDOW <= self.pending.len() {
            let window = &self.pending[offset..offset + VAD_WINDOW];
            self.vad.accept_waveform(window);
            self.kept.extend(window);
            self.accepted += VAD_WINDOW;
            offset += VAD_WINDOW;
        }
        self.pending.drain(..offset);
        if self.kept.len() > KEPT {
            let excess = self.kept.len() - KEPT;
            self.kept.drain(..excess);
            self.kept_from += excess;
        }

        let mut heard = Vec::new();
        while let Some(segment) = self.vad.front() {
            self.vad.pop();
            let audio = self.with_margins(segment.start(), segment.samples());
            let audio = audio.as_slice();
            let start = Instant::now();
            let text = self.transcriber.transcribe(audio).unwrap_or_else(|e| {
                eprintln!("[stt] {e:#}");
                String::new()
            });
            let (peak, rms) = level_db(audio);
            println!(
                "[stt] \"{}\" ({:.1}s audio, peak {:.0} dB, rms {:.0} dB, {} ms)",
                text,
                audio.len() as f32 / SAMPLE_RATE as f32,
                peak,
                rms,
                start.elapsed().as_millis()
            );
            if let Some(dir) = &self.recordings {
                self.segment_index += 1;
                let path = dir.join(format!("{:03}.wav", self.segment_index));
                if let Err(e) = std::fs::write(&path, wav(audio, SAMPLE_RATE as u32)) {
                    eprintln!("[mic] couldn't save {}: {e}", path.display());
                }
            }
            if !text.is_empty() {
                heard.push(text);
            }
        }
        heard
    }
}

impl Listener {
    /// A piece of speech the VAD found (its start in its own count, and its audio), with the
    /// mic audio around it: MARGIN_BEFORE before, MARGIN_AFTER after, as far as there is.
    fn with_margins(&self, start: i32, samples: &[f32]) -> Vec<f32> {
        let start = self.vad_origin + start.max(0) as usize;
        let from = start.saturating_sub(MARGIN_BEFORE).max(self.kept_from);
        let to = (start + samples.len() + MARGIN_AFTER).min(self.accepted);
        if from >= to || start < self.kept_from {
            return samples.to_vec();
        }
        self.kept.range(from - self.kept_from..to - self.kept_from).copied().collect()
    }

    /// True while the VAD thinks someone is talking (it needs ~0.25 s of speech to switch on).
    pub fn speech_active(&self) -> bool {
        self.vad.detected()
    }
}

/// Runs forever on the calling thread, sending each finished utterance's text to `out`.
///
/// With barge-in on, the mic stays live while the mate talks: as soon as the player starts
/// speaking, the mate is cut off (queued audio in the game, pending TTS and the LLM reply).
/// Opens the desktop default (ALSA's PipeWire bridge on CachyOS), leaving device selection
/// and live default-device changes to KDE/WirePlumber. Errors return to main's reconnect loop.
pub fn run(
    cfg: Config, transcriber: Transcriber, speaking: Speaking, link: GameLink,
    out: Sender<String>, on_ready: impl FnOnce(),
) -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("no microphone found"))?;
    let supported = device.default_input_config()?;
    let mic_rate = supported.sample_rate().0 as i32;
    let (tx, rx) = mpsc::channel::<MicChunk>();
    let stream = build_input_stream(&device, &supported, tx)?;
    stream.play()?;
    println!(
        "[mic] using '{}' at {} Hz, {} channel(s), {:?}",
        device.name().unwrap_or_default(),
        mic_rate,
        supported.channels(),
        supported.sample_format()
    );

    let (barge_in, mute_while_speaking) = (cfg.mic.barge_in, cfg.mic.mute_while_speaking);
    let name = cfg.mate.name.clone();
    let mut listener = Listener::new(cfg, transcriber, mic_rate)?;
    // Let the reconnect loop reset its one-error-per-outage log suppression.
    on_ready();
    for chunk in rx {
        let chunk = chunk.map_err(|e| anyhow!("microphone stream failed: {e}"))?;
        let mate_talking = speaking.is_speaking();
        // (muted in the game: nothing heard, and he isn't cut off; the listener starts over,
        // without counting the sound as talked over him)
        if link.mic_muted.load(std::sync::atomic::Ordering::Relaxed) {
            listener.feed(Vec::new(), true);
            continue;
        }
        let heard = listener.feed(chunk, !barge_in && mute_while_speaking && mate_talking);
        if barge_in && mate_talking && listener.speech_active() {
            println!("[mic] you talked over {name}, cutting them off");
            speaking.interrupt();
            link.send_json(&json!({"type": "stop_audio"}));
        }
        for text in heard {
            if out.send(text).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}
/// Peak and RMS level in dBFS.
fn level_db(samples: &[f32]) -> (f32, f32) {
    if samples.is_empty() {
        return (-100.0, -100.0);
    }
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    let db = |x: f32| 20.0 * x.max(1e-5).log10();
    (db(peak), db(rms))
}

type MicChunk = std::result::Result<Vec<f32>, cpal::StreamError>;

fn build_input_stream(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    tx: Sender<MicChunk>,
) -> Result<cpal::Stream> {
    let config = supported.config();
    let channels = config.channels as usize;
    let errors = tx.clone();
    let err_fn = move |err| {
        let _ = errors.send(Err(err));
    };

    fn to_mono<T: Copy>(data: &[T], channels: usize, conv: impl Fn(T) -> f32) -> Vec<f32> {
        data.chunks(channels)
            .map(|frame| frame.iter().map(|&s| conv(s)).sum::<f32>() / channels as f32)
            .collect()
    }

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let _ = tx.send(Ok(to_mono(data, channels, |s| s)));
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                let _ = tx.send(Ok(to_mono(data, channels, |s| s as f32 / i16::MAX as f32)));
            },
            err_fn,
            None,
        )?,
        other => bail!("unsupported microphone sample format {other:?}"),
    };
    Ok(stream)
}
