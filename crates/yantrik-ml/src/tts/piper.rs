//! Piper neural TTS — natural-sounding voice via subprocess.
//!
//! Uses the `piper` binary (pre-built, ~5MB) with ONNX voice models (~22MB).
//! No Rust dependencies needed — pure subprocess call.
//!
//! Pipeline: text → piper binary → raw PCM → aplay/paplay
//!
//! Install: download from https://github.com/rhasspy/piper/releases
//! Models: https://huggingface.co/rhasspy/piper-voices
//!
//! ```rust,ignore
//! let tts = PiperTTS::new("/opt/yantrik/models/tts/piper", "/opt/yantrik/models/tts/en_US-lessac-medium.onnx")?;
//! tts.speak("Hello world!", None)?;
//! ```

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Piper neural TTS engine — subprocess-based.
pub struct PiperTTS {
    piper_bin: PathBuf,
    model_path: PathBuf,
    sample_rate: u32,
}

impl PiperTTS {
    /// Create a new Piper TTS engine.
    ///
    /// `piper_bin`: path to the `piper` binary
    /// `model_path`: path to the ONNX voice model (.onnx file)
    pub fn new(piper_bin: &Path, model_path: &Path) -> Result<Self> {
        if !piper_bin.exists() {
            return Err(anyhow::anyhow!(
                "Piper binary not found at {}. Download from https://github.com/rhasspy/piper/releases",
                piper_bin.display()
            ));
        }
        if !model_path.exists() {
            return Err(anyhow::anyhow!(
                "Piper voice model not found at {}",
                model_path.display()
            ));
        }

        // Detect sample rate from model config (model.onnx.json)
        let config_path = PathBuf::from(format!("{}.json", model_path.display()));
        let sample_rate = if config_path.exists() {
            read_sample_rate(&config_path).unwrap_or(22050)
        } else {
            22050 // Piper default
        };

        tracing::info!(
            bin = %piper_bin.display(),
            model = %model_path.display(),
            sample_rate,
            "PiperTTS initialized"
        );

        Ok(Self {
            piper_bin: piper_bin.to_path_buf(),
            model_path: model_path.to_path_buf(),
            sample_rate,
        })
    }

    /// Auto-detect Piper binary and model from a directory.
    ///
    /// Searches for `piper` binary and `*.onnx` model file.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let piper_bin = find_piper_binary(dir)?;
        let model_path = find_onnx_model(dir)?;
        Self::new(&piper_bin, &model_path)
    }

    /// Synthesize text to raw PCM samples (i16, mono).
    pub fn synthesize_raw(&self, text: &str) -> Result<Vec<u8>> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Set LD_LIBRARY_PATH to piper's directory for shared libs
        let lib_dir = self.piper_bin.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let output = std::process::Command::new(&self.piper_bin)
            .args([
                "--model", &self.model_path.to_string_lossy(),
                "--output-raw",
            ])
            .env("LD_LIBRARY_PATH", &lib_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    use std::io::Write;
                    let _ = stdin.write_all(text.as_bytes());
                }
                drop(child.stdin.take());
                child.wait_with_output()
            })
            .map_err(|e| anyhow::anyhow!("Piper failed: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Piper error: {stderr}"));
        }

        Ok(output.stdout)
    }

    /// Speak text through system audio (blocking).
    ///
    /// Drop-in replacement for TTSEngine::speak().
    pub fn speak(&self, text: &str, params: Option<&crate::types::VoiceParams>) -> Result<()> {
        if text.trim().is_empty() {
            return Ok(());
        }

        // Split long text into sentences for lower latency
        let sentences = split_sentences(text);
        for sentence in &sentences {
            self.speak_sentence(sentence, params)?;
        }
        Ok(())
    }

    /// Speak a single sentence.
    fn speak_sentence(&self, text: &str, params: Option<&crate::types::VoiceParams>) -> Result<()> {
        // Build piper args with optional speed/length scale
        let mut args = vec![
            "--model".to_string(),
            self.model_path.to_string_lossy().to_string(),
            "--output-raw".to_string(),
        ];

        // Map voice params to piper's length_scale (inverse of rate)
        if let Some(p) = params {
            if (p.rate - 1.0).abs() > 0.05 {
                let length_scale = 1.0 / p.rate;
                args.push("--length-scale".into());
                args.push(format!("{:.2}", length_scale));
            }
        }

        // Set LD_LIBRARY_PATH for piper shared libs
        let lib_dir = self.piper_bin.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        // Pipe text → piper → aplay (streaming, low latency)
        let piper = std::process::Command::new(&self.piper_bin)
            .args(&args)
            .env("LD_LIBRARY_PATH", &lib_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Piper spawn: {e}"))?;

        // Feed text to piper stdin
        let mut piper = piper;
        if let Some(ref mut stdin) = piper.stdin {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        drop(piper.stdin.take());

        // Pipe piper stdout → aplay (or paplay)
        let piper_stdout = piper.stdout.take()
            .ok_or_else(|| anyhow::anyhow!("No piper stdout"))?;

        let player_result = std::process::Command::new("aplay")
            .args([
                "-r", &self.sample_rate.to_string(),
                "-f", "S16_LE",
                "-t", "raw",
                "-q",
                "-",
            ])
            .stdin(piper_stdout)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        match player_result {
            Ok(s) if s.success() => {
                let _ = piper.wait();
            }
            _ => {
                // aplay failed — synthesize to file instead
                let raw = self.synthesize_raw(text)?;
                if !raw.is_empty() {
                    // Private scratch, not a fixed name in /tmp another account could pre-plant.
                    let tmp = crate::private_dir::scratch_file_string("yantrik-piper.wav")?;
                    let tmp = tmp.as_str();
                    write_wav_from_raw(tmp, &raw, self.sample_rate)?;
                    play_wav(tmp)?;
                    let _ = std::fs::remove_file(tmp);
                }
                let _ = piper.wait();
            }
        }

        Ok(())
    }

    /// Check if currently speaking.
    pub fn is_speaking(&self) -> bool {
        false
    }

    /// Get the sample rate of the loaded model.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Read sample rate from piper model config JSON.
fn read_sample_rate(config_path: &Path) -> Option<u32> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("audio")?.get("sample_rate")?.as_u64().map(|r| r as u32)
}

/// Find the piper binary in a directory or system PATH.
fn find_piper_binary(dir: &Path) -> Result<PathBuf> {
    // Check in directory
    for name in &["piper", "piper-linux-x86_64", "piper-linux"] {
        let path = dir.join(name);
        if path.exists() {
            return Ok(path);
        }
    }
    // Check system PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg("piper")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    // Check common locations
    for path in &["/usr/local/bin/piper", "/opt/yantrik/bin/piper", "/usr/bin/piper"] {
        if Path::new(path).exists() {
            return Ok(PathBuf::from(path));
        }
    }
    Err(anyhow::anyhow!("Piper binary not found. Install from https://github.com/rhasspy/piper/releases"))
}

/// Find an ONNX voice model in a directory.
fn find_onnx_model(dir: &Path) -> Result<PathBuf> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".onnx") && !name.contains("kokoro") {
                return Ok(path);
            }
        }
    }
    Err(anyhow::anyhow!("No Piper .onnx voice model found in {}", dir.display()))
}

/// Split text into sentences.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        current.push(c);
        if matches!(c, '.' | '!' | '?' | '\n') && current.trim().len() > 1 {
            sentences.push(current.trim().to_string());
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }
    if sentences.is_empty() {
        sentences.push(text.to_string());
    }
    sentences
}

/// Write raw i16 PCM as WAV.
fn write_wav_from_raw(path: &str, raw: &[u8], sample_rate: u32) -> Result<()> {
    use std::io::Write;
    let data_size = raw.len() as u32;
    // Only ever a scratch file: never written through a link or second name left there.
    let mut f = crate::private_dir::create_private_file(std::path::Path::new(path))?;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_size).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&(sample_rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    f.write_all(raw)?;
    Ok(())
}

/// Play a WAV file through system audio.
fn play_wav(path: &str) -> Result<()> {
    for (player, args) in &[
        ("aplay", vec!["-q", path]),
        ("paplay", vec![path]),
        ("ffplay", vec!["-nodisp", "-autoexit", "-loglevel", "quiet", path]),
    ] {
        if let Ok(s) = std::process::Command::new(player)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            if s.success() { return Ok(()); }
        }
    }
    Err(anyhow::anyhow!("No audio player found"))
}
