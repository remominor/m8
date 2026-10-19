//! Breeze TTS with a saved server-side voice profile. Delivery instructions and supported
//! sounds come from the existing dialogue pipeline. PCM audio goes straight to the game
//! as it arrives; neither a local reference clip nor a readiness check is needed.

use crate::config::Config;
use crate::link::GameLink;
use crate::site::Trace;
use crate::Speaking;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::io::Read;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Work for the TTS thread, tagged with the `Speaking` generation it was produced for, so
/// lines queued before the player talked over the mate get dropped.
pub enum SpeechJob {
    /// A sentence, and how to say it (may be empty).
    Sentence(String, String, u64),
    /// The current reply is finished.
    End(u64),
}

pub struct Voice {
    url: String,
    agent: ureq::Agent,
    voice: String,
    guidance_scale: f32,
    seed: u64,
    pub sample_rate: u32,
}

impl Voice {
    /// Initialize without probing /health: synthesis can load an unloaded Breeze model.
    pub fn load(cfg: &Config) -> Result<Self> {
        if cfg.mate.voice_sample_rate == 0 {
            bail!("mate.voice_sample_rate must be positive");
        }
        let base = cfg.mate.voice_server.trim_end_matches('/');
        let url = if base.ends_with("/v1/audio/speech") {
            base.to_string()
        } else {
            format!("{base}/v1/audio/speech")
        };
        Ok(Voice {
            url,
            agent: ureq::AgentBuilder::new().build(),
            voice: cfg.mate.voice.clone(),
            guidance_scale: cfg.mate.voice_guidance_scale,
            seed: cfg.mate.voice_seed,
            sample_rate: cfg.mate.voice_sample_rate,
        })
    }

    /// Speak one sentence, forwarding samples immediately. A stale generation stops retries
    /// and drops the streaming connection; `on_chunk` can also stop playback early.
    pub fn stream(
        &self,
        text: &str,
        how: &str,
        is_current: impl Fn() -> bool,
        on_chunk: impl FnMut(&[f32]) -> bool,
    ) -> Result<()> {
        let mut body = json!({
            "input": text,
            "voice": self.voice,
            "guidance_scale": if how.is_empty() { 1.0 } else { self.guidance_scale },
            "seed": self.seed,
            "stream": true,
            "response_format": "pcm",
        });
        if !how.is_empty() {
            body["instructions"] = json!(how);
        }
        // 409: still finishing a line he was cut off in. Keep the original retry budget.
        let mut retries = 0;
        let response = loop {
            if !is_current() {
                return Ok(());
            }
            match self.agent.post(&self.url).send_json(&body) {
                Err(ureq::Error::Status(409, _)) if retries < 20 => {
                    retries += 1;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(ureq::Error::Status(code, response)) => {
                    bail!(
                        "Breeze at {} returned {code}: {}",
                        self.url,
                        response.into_string().unwrap_or_default()
                    );
                }
                other => {
                    break other.with_context(|| format!("Breeze synthesis at {}", self.url))?
                }
            }
        };
        if !is_current() {
            return Ok(());
        }
        let rate = response
            .header("X-Sample-Rate")
            .ok_or_else(|| anyhow!("Breeze sent no X-Sample-Rate"))?
            .parse::<u32>()
            .context("invalid Breeze X-Sample-Rate")?;
        if rate != self.sample_rate {
            bail!("Breeze sample rate {rate} does not match mate.voice_sample_rate {} (game handshake)", self.sample_rate);
        }
        if response.header("X-Sample-Format") != Some("s16le") {
            bail!(
                "Breeze must return X-Sample-Format: s16le, got {:?}",
                response.header("X-Sample-Format")
            );
        }
        stream_pcm(response.into_reader(), is_current, on_chunk)
    }
}

/// Reads can end halfway through a PCM16 sample. Retain that byte for the next read.
fn stream_pcm(
    mut reader: impl Read,
    is_current: impl Fn() -> bool,
    mut on_chunk: impl FnMut(&[f32]) -> bool,
) -> Result<()> {
    let (mut buffer, mut held) = ([0u8; 4800], 0);
    let mut received_audio = false;
    loop {
        if !is_current() {
            return Ok(());
        }
        let read = reader.read(&mut buffer[held..])?;
        if !is_current() {
            return Ok(());
        }
        if read == 0 {
            if held != 0 {
                bail!("Breeze returned truncated PCM16 audio (one trailing byte)");
            }
            if !received_audio {
                bail!("Breeze returned no PCM audio");
            }
            return Ok(());
        }
        let whole = (held + read) / 2 * 2;
        let samples: Vec<f32> = buffer[..whole]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        buffer.copy_within(whole..held + read, 0);
        held = held + read - whole;
        received_audio |= !samples.is_empty();
        if !samples.is_empty() && !on_chunk(&samples) {
            return Ok(());
        }
    }
}

pub fn run(
    voice: Voice,
    link: GameLink,
    speaking: Speaking,
    trace: Trace,
    jobs: Receiver<SpeechJob>,
) -> Result<()> {
    let sample_rate = voice.sample_rate as f32;
    // (the line whose audio went to the game last, and when that audio runs out: a new line
    // takes over from an older one still playing as its first audio arrives, not before)
    let mut audio_generation = None;
    let mut audio_until = Instant::now();
    for job in jobs {
        match job {
            SpeechJob::Sentence(_, _, generation) if generation != speaking.generation() => {
                // Queued before the player talked over the mate.
            }
            SpeechJob::End(generation) => {
                if generation == speaking.generation() {
                    link.send_json(&json!({"type": "audio_end"}));
                }
                speaking.end_line();
            }
            SpeechJob::Sentence(text, how, generation) => {
                link.say(&text);
                let start = Instant::now();
                let mut first_chunk_ms = None;
                let result = voice.stream(
                    &text,
                    &how,
                    || speaking.generation() == generation,
                    |samples| {
                        if speaking.generation() != generation {
                            return false;
                        }
                        first_chunk_ms.get_or_insert(start.elapsed().as_millis());
                        if audio_generation != Some(generation) {
                            if Instant::now() < audio_until {
                                link.send_json(&json!({"type": "stop_audio"}));
                            }
                            audio_generation = Some(generation);
                        }
                        audio_until = audio_until.max(Instant::now())
                            + Duration::from_secs_f32(samples.len() as f32 / sample_rate);
                        let pcm: Vec<u8> = samples
                            .iter()
                            .flat_map(|&s| ((s.clamp(-1.0, 1.0) * 32768.0) as i16).to_le_bytes())
                            .collect();
                        speaking
                            .extend(Duration::from_secs_f32(samples.len() as f32 / sample_rate));
                        link.send_audio(&pcm);
                        true
                    },
                );
                if let Err(e) = &result {
                    eprintln!("[tts] failed on \"{text}\": {e:#}");
                }
                trace.add(
                    "voice",
                    json!({
                        "text": text,
                        "how": how,
                        "first_audio_ms": first_chunk_ms.map(|ms| ms as u64),
                        "ms": start.elapsed().as_millis() as u64,
                        "cut_off": speaking.generation() != generation,
                        "error": result.as_ref().err().map(|e| format!("{e:#}")),
                    }),
                );
                println!(
                    "[tts] [{how}] \"{}\" (first audio {} ms, total {} ms)",
                    text,
                    first_chunk_ms.unwrap_or(0),
                    start.elapsed().as_millis()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    };

    fn voice(server: &MockHttp) -> Voice {
        let mut cfg = config();
        cfg.mate.voice_server = server.url.clone();
        Voice::load(&cfg).unwrap()
    }

    #[test]
    fn saved_profile_json_and_unloaded_health_do_not_block_startup() {
        let server = MockHttp::new(|request, stream| {
            if request.path == "/health" {
                reply(stream, 503, &[], br#"{"status":"unloaded"}"#);
            } else {
                reply(stream, 200, PCM_HEADERS, &[0, 0]);
            }
        });
        // Establish that the fixture really represents an unloaded server.
        assert!(matches!(
            ureq::get(&format!("{}/health", server.url)).call(),
            Err(ureq::Error::Status(503, _))
        ));
        assert_eq!(server.request().path, "/health");
        let voice = voice(&server);
        assert_eq!(voice.sample_rate, 24000);
        assert!(server.requests.try_recv().is_err()); // load performs no HTTP calls
        for how in ["yelling over gunfire", ""] {
            voice.stream("Cover me!", how, || true, |_| true).unwrap();
            let request = server.request();
            assert_eq!(request.method, "POST");
            assert_eq!(request.path, "/v1/audio/speech");
            assert_eq!(request.headers["content-type"], "application/json");
            let body = request.json();
            assert_eq!(body["input"], "Cover me!");
            assert_eq!(body["voice"], "soldier");
            assert_eq!(body["seed"], 42);
            assert_eq!(body["stream"], true);
            assert_eq!(body["response_format"], "pcm");
            assert!(body.get("ref_audio").is_none());
            assert!(body.get("ref_text").is_none());
            if how.is_empty() {
                assert!(body.get("instructions").is_none());
                assert_eq!(body["guidance_scale"], 1.0);
            } else {
                assert_eq!(body["instructions"], how);
                assert_eq!(body["guidance_scale"], 4.0);
            }
        }
    }

    #[test]
    fn configured_voice_guidance_and_url_are_used() {
        let server = MockHttp::new(|_, stream| reply(stream, 200, PCM_HEADERS, &[0, 0]));
        for suffix in ["", "/", "/v1/audio/speech", "/v1/audio/speech/"] {
            let mut cfg = config();
            cfg.mate.voice_server = format!("{}{suffix}", server.url);
            cfg.mate.voice = "clone:soldier".into();
            cfg.mate.voice_guidance_scale = 3.5;
            Voice::load(&cfg)
                .unwrap()
                .stream("Move!", "urgent", || true, |_| true)
                .unwrap();
            let request = server.request();
            assert_eq!(request.path, "/v1/audio/speech");
            assert_eq!(request.json()["voice"], "clone:soldier");
            assert_eq!(request.json()["guidance_scale"], 3.5);
        }
    }

    #[test]
    fn invalid_audio_headers_fail_before_any_audio() {
        let cases = [
            vec![],
            vec![("X-Sample-Rate", "invalid"), ("X-Sample-Format", "s16le")],
            vec![("X-Sample-Rate", "48000"), ("X-Sample-Format", "s16le")],
            vec![("X-Sample-Rate", "24000")],
            vec![("X-Sample-Rate", "24000"), ("X-Sample-Format", "f32le")],
        ];
        for headers in cases {
            let server = MockHttp::new(move |_, stream| reply(stream, 200, &headers, &[0, 0]));
            let result =
                voice(&server).stream("Hello", "", || true, |_| panic!("invalid audio forwarded"));
            assert!(result.is_err());
        }
        let mut cfg = config();
        cfg.mate.voice_sample_rate = 0;
        assert!(Voice::load(&cfg).is_err());
    }

    #[test]
    fn odd_http_chunks_stream_before_completion() {
        let (received, wait) = mpsc::channel();
        let server = MockHttp::new(move |_, stream| {
            chunked_head(stream);
            chunk(stream, &[0]);
            chunk(stream, &[128, 255]); // -32768 plus half of 32767
                                        // Server cannot finish until the client forwards the first complete sample.
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            chunk(stream, &[127, 0, 0]);
            chunk(stream, &[]);
        });
        let mut samples = Vec::new();
        voice(&server)
            .stream(
                "Hello",
                "",
                || true,
                |part| {
                    samples.extend_from_slice(part);
                    received.send(()).unwrap();
                    true
                },
            )
            .unwrap();
        assert_eq!(samples, [-1.0, 32767.0 / 32768.0, 0.0]);
    }

    #[test]
    fn pcm_decoder_handles_single_byte_reads_and_truncation() {
        struct SingleByte<'a>(&'a [u8]);
        impl Read for SingleByte<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Ok(0);
                }
                buffer[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let bytes: Vec<_> = [i16::MIN, -1, 0, 1, i16::MAX]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        let mut samples = Vec::new();
        stream_pcm(
            SingleByte(&bytes),
            || true,
            |part| {
                samples.extend_from_slice(part);
                true
            },
        )
        .unwrap();
        assert_eq!(
            samples,
            [-1.0, -1.0 / 32768.0, 0.0, 1.0 / 32768.0, 32767.0 / 32768.0]
        );
        assert!(stream_pcm(SingleByte(&bytes[..3]), || true, |_| true).is_err());
    }

    #[test]
    fn inference_errors_and_empty_streams_are_visible() {
        let server = MockHttp::new(|_, stream| reply(stream, 500, &[], b"synthesis failed"));
        let error = voice(&server)
            .stream("Hello", "", || true, |_| true)
            .unwrap_err();
        assert!(error.to_string().contains("500"));
        assert!(error.to_string().contains("synthesis failed"));
        let empty = MockHttp::new(|_, stream| reply(stream, 200, PCM_HEADERS, &[]));
        assert!(voice(&empty)
            .stream("Hello", "", || true, |_| true)
            .unwrap_err()
            .to_string()
            .contains("no PCM audio"));
    }

    #[test]
    fn busy_retries_then_succeeds_and_exhaustion_is_visible() {
        let mut count = 0;
        let server = MockHttp::new(move |_, stream| {
            count += 1;
            if count <= 2 {
                reply(stream, 409, &[], b"busy");
            } else {
                reply(stream, 200, PCM_HEADERS, &[0, 0]);
            }
        });
        voice(&server)
            .stream("Hello", "", || true, |_| true)
            .unwrap();
        for _ in 0..3 {
            server.request();
        }
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let busy = MockHttp::new(move |_, stream| {
            calls.fetch_add(1, Ordering::SeqCst);
            reply(stream, 409, &[], b"still busy");
        });
        let error = voice(&busy)
            .stream("Hello", "", || true, |_| true)
            .unwrap_err();
        assert!(error.to_string().contains("409"));
        assert!(error.to_string().contains("still busy"));
        assert_eq!(count.load(Ordering::SeqCst), 21);
    }

    #[test]
    fn cancellation_stops_busy_retries_and_stale_requests() {
        let active = Arc::new(AtomicBool::new(true));
        let cancel = active.clone();
        let server = MockHttp::new(move |_, stream| {
            cancel.store(false, Ordering::SeqCst);
            reply(stream, 409, &[], b"busy");
        });
        let voice = voice(&server);
        voice
            .stream(
                "Hello",
                "",
                || active.load(Ordering::SeqCst),
                |_| panic!("stale audio"),
            )
            .unwrap();
        server.request();
        voice
            .stream("Old sentence", "", || false, |_| panic!("stale audio"))
            .unwrap();
        assert!(server.requests.try_recv().is_err());
    }

    #[test]
    fn interrupted_stream_drops_connection_without_forwarding_later_audio() {
        let (cancelled, wait) = mpsc::channel();
        let server = MockHttp::new(move |_, stream| {
            chunked_head(stream);
            chunk(stream, &[0, 0]);
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0); // client closed the body
        });
        let current = AtomicBool::new(true);
        let mut calls = 0;
        voice(&server)
            .stream(
                "Hello",
                "",
                || current.load(Ordering::SeqCst),
                |_| {
                    calls += 1;
                    current.store(false, Ordering::SeqCst);
                    cancelled.send(()).unwrap();
                    true
                },
            )
            .unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn speech_jobs_forward_exact_pcm_and_manage_generations_and_queue() {
        let server = MockHttp::new(|request, stream| {
            let mut pcm = vec![0, 128, 255, 127, 1, 0];
            if request.json()["input"] == "First" {
                pcm.resize(48000, 0);
            }
            reply(stream, 200, PCM_HEADERS, &pcm);
        });
        let voice = voice(&server);
        let (link, mut peer) = GameLink::test_pair();
        let speaking = Speaking::new();
        let dir = TestDir::new();
        let trace = Trace::new(&dir.0);
        let (tx, rx) = mpsc::channel();
        let actor = speaking.clone();
        let thread = std::thread::spawn(move || run(voice, link, actor, trace, rx).unwrap());
        tx.send(SpeechJob::Sentence("First".into(), "".into(), 0))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&frame(&mut peer).1).unwrap()["type"],
            "say"
        );
        let mut audio = Vec::new();
        while audio.len() < 48000 {
            let (kind, body) = frame(&mut peer);
            assert_eq!(kind, 2);
            audio.extend(body);
        }
        let mut expected = vec![0, 128, 255, 127, 1, 0];
        expected.resize(48000, 0);
        assert_eq!(audio, expected);
        // A new generation starts while the old line still has queued playback.
        speaking.interrupt();
        speaking.begin_line();
        tx.send(SpeechJob::Sentence("Stale".into(), "".into(), 0))
            .unwrap();
        tx.send(SpeechJob::End(0)).unwrap();
        tx.send(SpeechJob::Sentence("Second".into(), "".into(), 1))
            .unwrap();
        tx.send(SpeechJob::End(1)).unwrap();
        drop(tx);
        thread.join().unwrap();
        let mut kinds = Vec::new();
        for _ in 0..4 {
            let (kind, body) = frame(&mut peer);
            if kind == 1 {
                kinds.push(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["type"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                );
            } else {
                assert_eq!(kind, 2);
            }
        }
        assert_eq!(kinds, ["say", "stop_audio", "audio_end"]);
        assert!(speaking.quiet_from().is_some());
        server.request();
        server.request();
        assert!(server.requests.try_recv().is_err());
    }
}
