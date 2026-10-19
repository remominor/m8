use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone)]
pub struct Config {
    pub mate: MateConfig,
    pub mic: MicConfig,
    pub stt: SttConfig,
    pub kev: KevConfig,
    pub llm: LlmConfig,
    pub models: ModelsConfig,
    pub game: GameConfig,
    /// Folder the config file lives in; relative paths are resolved against it.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Deserialize, Clone)]
pub struct MateConfig {
    pub name: String,
    /// The persona's folder: system_prompt.txt (who the mate is), combat_prompt.txt and
    /// radio_prompt.txt (added while the game says so; either may be missing) and levels/ (each
    /// level's story, by the name the game gives it). The config's folder if unset.
    #[serde(default)]
    pub persona: Option<PathBuf>,
    /// Base URL of the Breeze TTS API.
    pub voice_server: String,
    /// A saved server-side profile; no local reference audio is needed.
    #[serde(default = "default_voice")]
    pub voice: String,
    #[serde(default = "default_sample_rate")]
    pub voice_sample_rate: u32,
    #[serde(default = "default_guidance")]
    pub voice_guidance_scale: f32,
    /// Its noise seed, so every line comes out in the same voice.
    #[serde(default = "default_seed")]
    pub voice_seed: u64,
}

#[derive(Deserialize, Clone)]
pub struct MicConfig {
    /// Talking over the mate cuts it off. Takes precedence over `mute_while_speaking`.
    #[serde(default = "yes")]
    pub barge_in: bool,
    pub mute_while_speaking: bool,
    /// Save every recognised clip to `recordings/` for debugging.
    #[serde(default)]
    pub save_recordings: bool,
    pub vad_threshold: f32,
    pub end_of_speech_silence: f32,
}

/// Speech to text: an OpenAI-style /v1/audio/transcriptions (listen.rs).
#[derive(Deserialize, Clone)]
pub struct SttConfig {
    pub url: String,
    /// The `model` it's sent, for servers that want one.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct KevConfig {
    pub url: String,
    /// Total System One request budget, independent of dialogue and vision.
    #[serde(default = "default_kev_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_kev_model")]
    pub model: String,
}

#[derive(Deserialize, Clone)]
pub struct LlmConfig {
    pub url: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    pub temperature: f32,
}

#[derive(Deserialize, Clone)]
pub struct ModelsConfig {
    /// Silero VAD, which finds where speech starts and ends in the mic's audio.
    pub silero_vad: PathBuf,
}

#[derive(Deserialize, Clone)]
pub struct GameConfig {
    pub port: u16,
}

impl Config {
    /// Loads the TOML file and makes every model path absolute relative to it.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&text).context("parsing config")?;
        anyhow::ensure!(
            cfg.kev.timeout_ms > 0,
            "kev.timeout_ms must be greater than zero"
        );
        // Model routing uses exact IDs; accidental whitespace must not become part of one.
        for model in [&mut cfg.llm.model, &mut cfg.kev.model]
            .into_iter()
            .chain(cfg.stt.model.iter_mut())
        {
            *model = model.trim().to_string();
        }
        let base = path.parent().unwrap_or(Path::new("."));
        cfg.base_dir = base.to_path_buf();
        for p in std::iter::once(&mut cfg.models.silero_vad).chain(cfg.mate.persona.as_mut()) {
            if p.is_relative() {
                *p = base.join(&*p);
            }
        }
        Ok(cfg)
    }
}

impl Config {
    /// The persona's folder (`persona`), else the config's.
    pub fn persona_dir(&self) -> PathBuf {
        self.mate
            .persona
            .clone()
            .unwrap_or_else(|| self.base_dir.clone())
    }
}

fn yes() -> bool {
    true
}

fn default_voice() -> String {
    "soldier".into()
}
fn default_sample_rate() -> u32 {
    24000
}
fn default_guidance() -> f32 {
    4.0
}
fn default_seed() -> u64 {
    42
}
fn default_kev_timeout() -> u64 {
    5000
}
fn default_kev_model() -> String {
    "clef-flash-artemis".into()
}
fn default_llm_model() -> String {
    "gemma4-12b-mtp-vision".into()
}

/// sherpa-onnx wants paths as `String`s.
pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    #[test]
    fn system_one_timeout_defaults_is_configurable_and_rejects_zero() {
        let mut document: toml::Value = toml::from_str(crate::test_support::CONFIG).unwrap();
        assert_eq!(
            toml::from_str::<Config>(crate::test_support::CONFIG)
                .unwrap()
                .kev
                .timeout_ms,
            5000
        );
        document["kev"]
            .as_table_mut()
            .unwrap()
            .insert("timeout_ms".into(), toml::Value::Integer(250));
        let dir = TestDir::new();
        let path = dir.0.join("m8.toml");
        std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
        assert_eq!(Config::load(&path).unwrap().kev.timeout_ms, 250);
        document["kev"]["timeout_ms"] = toml::Value::Integer(0);
        std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
        assert!(Config::load(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("timeout_ms"));
    }

    #[test]
    fn model_routing_ids_ignore_surrounding_whitespace() {
        let dir = TestDir::new();
        let mut document: toml::Value = toml::from_str(crate::test_support::CONFIG).unwrap();
        for section in ["llm", "kev", "stt"] {
            let model = document[section]["model"].as_str().unwrap().to_string();
            document[section]["model"] = toml::Value::String(format!("  {model} \t"));
        }
        let path = dir.0.join("m8.toml");
        std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.llm.model, "gemma4-12b-mtp-vision");
        assert_eq!(cfg.kev.model, "clef-flash-artemis");
        assert_eq!(cfg.stt.model.as_deref(), Some("parakeet"));
    }

    #[test]
    fn defaults_and_paths_need_no_local_voice_files() {
        let dir = TestDir::new();
        let mut document: toml::Value = toml::from_str(crate::test_support::CONFIG).unwrap();
        for field in [
            "voice",
            "voice_sample_rate",
            "voice_guidance_scale",
            "voice_seed",
        ] {
            document["mate"].as_table_mut().unwrap().remove(field);
        }
        document["llm"].as_table_mut().unwrap().remove("model");
        document["kev"].as_table_mut().unwrap().remove("model");
        let path = dir.0.join("m8.toml");
        std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.mate.voice, "soldier");
        assert_eq!(cfg.mate.voice_sample_rate, 24000);
        assert_eq!(cfg.mate.voice_guidance_scale, 4.0);
        assert_eq!(cfg.mate.voice_seed, 42);
        assert_eq!(cfg.llm.model, "gemma4-12b-mtp-vision");
        assert_eq!(cfg.kev.model, "clef-flash-artemis");
        assert_eq!(cfg.stt.model.as_deref(), Some("parakeet"));
        assert_eq!(cfg.models.silero_vad, dir.0.join("models/silero_vad.onnx"));
        assert_eq!(cfg.persona_dir(), dir.0.join("examples/kevin"));
        crate::speak::Voice::load(&cfg).unwrap();
    }
}
