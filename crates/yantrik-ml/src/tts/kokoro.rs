//! Kokoro neural TTS — direct ONNX inference via `ort` crate.
//!
//! Pipeline: Text → espeak-ng IPA → token IDs → ONNX model → 24kHz f32 audio
//!
//! Model: Kokoro 82M (88MB int8 ONNX from HuggingFace)
//! Voice: Style vectors loaded from .bin files (one per voice)
//!
//! No third-party TTS crate — drives ONNX Runtime directly.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SAMPLE_RATE: u32 = 24000;
const STYLE_DIM: usize = 256;
const MAX_PHONEME_LEN: usize = 510;

/// Kokoro neural TTS engine.
pub struct KokoroTTS {
    session: Mutex<ort::session::Session>,
    voices: HashMap<String, Vec<[f32; STYLE_DIM]>>,
    vocab: HashMap<char, i64>,
    default_voice: String,
    /// Whether input tensor is named "input_ids" (true) or "tokens" (false)
    uses_input_ids: bool,
}

unsafe impl Send for KokoroTTS {}
unsafe impl Sync for KokoroTTS {}

impl KokoroTTS {
    /// Load Kokoro TTS from a model directory containing:
    /// - `model.onnx` or `model_quantized.onnx`
    /// - `voices/` directory with `.bin` files (e.g., `af_heart.bin`)
    pub fn from_dir(model_dir: &Path) -> Result<Self> {
        // Find ONNX model file
        let model_path = find_model_file(model_dir)?;

        // Load ONNX session
        let session = ort::session::Session::builder()?
            .with_intra_threads(4)?
            .commit_from_file(&model_path)?;

        // Detect input tensor name
        let uses_input_ids = session.inputs.iter()
            .any(|i| i.name == "input_ids");

        // Load vocabulary
        let vocab = build_vocab();

        // Load voice style vectors
        let voices_dir = model_dir.join("voices");
        let mut voices = HashMap::new();
        if voices_dir.is_dir() {
            for entry in std::fs::read_dir(&voices_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("bin") {
                    let name = path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    match load_voice_bin(&path) {
                        Ok(styles) => {
                            tracing::debug!(voice = %name, vectors = styles.len(), "Loaded voice");
                            voices.insert(name, styles);
                        }
                        Err(e) => tracing::warn!(voice = %path.display(), err = %e, "Failed to load voice"),
                    }
                }
            }
        }

        let default_voice = if voices.contains_key("af_heart") {
            "af_heart".into()
        } else {
            voices.keys().next().cloned().unwrap_or_else(|| "af_heart".into())
        };

        tracing::info!(
            model = %model_path.display(),
            voices = voices.len(),
            default = %default_voice,
            "KokoroTTS initialized"
        );

        Ok(Self {
            session: Mutex::new(session),
            voices,
            vocab,
            default_voice,
            uses_input_ids,
        })
    }

    /// Synthesize text to f32 audio samples (24kHz mono).
    pub fn synthesize(&self, text: &str, voice: Option<&str>, speed: f32) -> Result<Vec<f32>> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 1. Text → IPA phonemes via espeak-ng
        let ipa = text_to_ipa(text)?;
        if ipa.is_empty() {
            return Ok(Vec::new());
        }

        // 2. IPA → Kokoro phonemes (apply replacement rules)
        let phonemes = ipa_to_kokoro(&ipa);

        // 3. Phonemes → token IDs via vocab lookup
        let mut token_ids: Vec<i64> = phonemes.chars()
            .filter_map(|c| self.vocab.get(&c).copied())
            .collect();

        // Truncate to max length
        if token_ids.len() > MAX_PHONEME_LEN {
            token_ids.truncate(MAX_PHONEME_LEN);
        }

        if token_ids.is_empty() {
            return Err(anyhow::anyhow!("No valid tokens after phonemization"));
        }

        // 4. Pad: [0, t1, t2, ..., tN, 0]
        let seq_len = token_ids.len() + 2;
        let mut padded = vec![0i64; seq_len];
        padded[1..seq_len - 1].copy_from_slice(&token_ids);

        // 5. Select voice style vector
        let voice_name = voice.unwrap_or(&self.default_voice);
        let style_vector = self.get_style_vector(voice_name, token_ids.len())?;

        // 6. Run ONNX inference
        let mut session = self.session.lock()
            .map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        let input_name = if self.uses_input_ids { "input_ids" } else { "tokens" };

        // Build input tensors using ort::Value::from_array
        let tokens_tensor = ort::value::Tensor::from_array(
            ([1usize, seq_len], padded)
        )?;
        let style_tensor = ort::value::Tensor::from_array(
            ([1usize, STYLE_DIM], style_vector)
        )?;
        let speed_tensor = ort::value::Tensor::from_array(
            ([1usize], vec![speed])
        )?;

        let outputs = session.run(ort::inputs![
            input_name => tokens_tensor,
            "style" => style_tensor,
            "speed" => speed_tensor,
        ]?)?;

        // 7. Extract audio samples
        let audio = outputs.iter().next()
            .ok_or_else(|| anyhow::anyhow!("No output tensor"))?
            .1
            .try_extract_tensor::<f32>()?;

        let samples: Vec<f32> = audio.as_slice()
            .ok_or_else(|| anyhow::anyhow!("Cannot get audio slice"))?
            .to_vec();

        tracing::debug!(
            text_len = text.len(),
            phonemes = phonemes.len(),
            tokens = token_ids.len(),
            samples = samples.len(),
            duration_s = samples.len() as f32 / SAMPLE_RATE as f32,
            "Kokoro synthesized"
        );

        Ok(samples)
    }

    /// Speak text through system audio (blocking).
    /// Drop-in replacement for TTSEngine::speak().
    pub fn speak(&self, text: &str, params: Option<&crate::types::VoiceParams>) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        let (voice, speed) = params
            .map(|p| {
                let v = if p.rate > 1.1 { "af_heart" }
                    else if p.rate > 0.95 { "af_bella" }
                    else { "af_sarah" };
                (v, p.rate)
            })
            .unwrap_or(("af_heart", 1.0));

        // For long text, split into sentences and synthesize each
        let sentences = split_sentences(text);
        for sentence in &sentences {
            let samples = self.synthesize(sentence, Some(voice), speed)?;
            if !samples.is_empty() {
                play_samples(&samples, SAMPLE_RATE)?;
            }
        }
        Ok(())
    }

    /// Check if currently speaking (always false — blocking API).
    pub fn is_speaking(&self) -> bool {
        false
    }

    /// Get style vector for a voice at a given token count.
    fn get_style_vector(&self, voice_name: &str, token_count: usize) -> Result<Vec<f32>> {
        let styles = self.voices.get(voice_name)
            .or_else(|| self.voices.values().next())
            .ok_or_else(|| anyhow::anyhow!("No voices loaded"))?;

        let idx = token_count.min(styles.len().saturating_sub(1));
        Ok(styles[idx].to_vec())
    }

    /// List available voice names.
    pub fn voices(&self) -> Vec<&str> {
        self.voices.keys().map(|s| s.as_str()).collect()
    }
}

// ── Phonemization ──────────────────────────────────────────────────

/// Convert text to IPA phonemes using espeak-ng subprocess.
fn text_to_ipa(text: &str) -> Result<String> {
    let output = std::process::Command::new("espeak-ng")
        .args(["--ipa", "-q", "--stdin", "-v", "en-us"])
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
        .map_err(|e| anyhow::anyhow!("espeak-ng not found: {e}. Install with: apt install espeak-ng"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("espeak-ng failed: {stderr}"));
    }

    let ipa = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(ipa)
}

/// Convert espeak-ng IPA to Kokoro phoneme format.
fn ipa_to_kokoro(ipa: &str) -> String {
    let mut result = ipa.to_string();

    // Apply replacements (longest first to avoid partial matches)
    let replacements = [
        ("a\u{361}ɪ", "I"),   // a͡ɪ → I
        ("a\u{361}ʊ", "W"),   // a͡ʊ → W
        ("d\u{361}ʒ", "ʤ"),   // d͡ʒ → ʤ
        ("e\u{361}ɪ", "A"),   // e͡ɪ → A
        ("t\u{361}ʃ", "ʧ"),   // t͡ʃ → ʧ
        ("ɔ\u{361}ɪ", "Y"),   // ɔ͡ɪ → Y
        ("o\u{361}ʊ", "O"),   // o͡ʊ → O
        ("ə\u{361}l", "ᵊl"), // ə͡l → ᵊl
        ("ɜːɹ", "ɜɹ"),
        ("ɪə", "iə"),
        ("ɚ", "əɹ"),
        ("e", "A"),
        ("r", "ɹ"),
        ("x", "k"),
        ("ç", "k"),
        ("ɐ", "ə"),
        ("ɬ", "l"),
        ("ʔ", "t"),
        ("ʲ", ""),
    ];

    for (from, to) in &replacements {
        result = result.replace(from, to);
    }

    // Remove length markers (ː) for American English
    result = result.replace('ː', "");

    // Remove tie bars
    result = result.replace('\u{361}', "");

    result
}

// ── Vocabulary ─────────────────────────────────────────────────────

/// Build the Kokoro phoneme vocabulary (char → token ID).
fn build_vocab() -> HashMap<char, i64> {
    let mut v = HashMap::new();
    // Punctuation & special
    v.insert('$', 0);  // pad/BOS/EOS
    v.insert(';', 1); v.insert(':', 2); v.insert(',', 3);
    v.insert('.', 4); v.insert('!', 5); v.insert('?', 6);
    v.insert('—', 9); v.insert('…', 10);
    v.insert('"', 11); v.insert('(', 12); v.insert(')', 13);
    v.insert('"', 14); v.insert('"', 15);
    // Space
    v.insert(' ', 16);
    // Latin letters used in IPA
    v.insert('A', 24); v.insert('I', 25); v.insert('O', 31);
    v.insert('Q', 33); v.insert('W', 39); v.insert('Y', 41);
    // IPA consonants
    v.insert('b', 44); v.insert('d', 46); v.insert('f', 48);
    v.insert('h', 50); v.insert('j', 52); v.insert('k', 53);
    v.insert('l', 54); v.insert('m', 55); v.insert('n', 56);
    v.insert('p', 58); v.insert('s', 61); v.insert('t', 62);
    v.insert('v', 64); v.insert('w', 65); v.insert('z', 68);
    // IPA vowels
    v.insert('æ', 71); v.insert('ð', 74); v.insert('ŋ', 78);
    v.insert('ɑ', 80); v.insert('ɔ', 81); v.insert('ʤ', 82);
    v.insert('ə', 83); v.insert('ɛ', 84); v.insert('ɜ', 86);
    v.insert('ɡ', 87); v.insert('ɪ', 88); v.insert('ɹ', 90);
    v.insert('ʃ', 92); v.insert('ʊ', 93); v.insert('ʌ', 94);
    v.insert('ʒ', 96); v.insert('ʧ', 97);
    v.insert('θ', 98);
    // Stress markers
    v.insert('ˈ', 100); v.insert('ˌ', 101);
    // Superscript
    v.insert('ᵊ', 103);
    v
}

// ── Voice loading ──────────────────────────────────────────────────

/// Load voice style vectors from a .bin file.
/// Format: raw little-endian f32, reshaped to [N, 256].
fn load_voice_bin(path: &Path) -> Result<Vec<[f32; STYLE_DIM]>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() % (STYLE_DIM * 4) != 0 {
        return Err(anyhow::anyhow!(
            "Voice file size {} not divisible by {} (256 * 4 bytes)",
            bytes.len(), STYLE_DIM * 4
        ));
    }

    let floats: Vec<f32> = bytes.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    let vectors: Vec<[f32; STYLE_DIM]> = floats.chunks_exact(STYLE_DIM)
        .map(|chunk| {
            let mut arr = [0f32; STYLE_DIM];
            arr.copy_from_slice(chunk);
            arr
        })
        .collect();

    Ok(vectors)
}

/// Find the ONNX model file in a directory.
fn find_model_file(dir: &Path) -> Result<PathBuf> {
    // Prefer quantized model (smaller, faster)
    for name in &["model_quantized.onnx", "model_q4f16.onnx", "model.onnx", "kokoro-v1.0.onnx"] {
        let path = dir.join(name);
        if path.exists() {
            return Ok(path);
        }
    }
    // Search for any .onnx file
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("onnx") {
            return Ok(path);
        }
    }
    Err(anyhow::anyhow!("No ONNX model found in {}", dir.display()))
}

// ── Text splitting ─────────────────────────────────────────────────

/// Split text into sentences for chunk-by-chunk synthesis.
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

// ── Audio playback ─────────────────────────────────────────────────

/// Play f32 audio samples through system audio.
fn play_samples(samples: &[f32], sample_rate: u32) -> Result<()> {
    // Private scratch, not a fixed name in /tmp that another account could plant a link at (and
    // have `File::create` truncate) or read — this is the person's speech.
    let tmp = crate::private_dir::scratch_file_string("yantrik-tts.wav")?;
    let tmp = tmp.as_str();
    write_wav(tmp, samples, sample_rate)?;

    for (player, args) in &[
        ("aplay", vec!["-q", tmp]),
        ("paplay", vec![tmp]),
        ("ffplay", vec!["-nodisp", "-autoexit", "-loglevel", "quiet", tmp]),
    ] {
        if let Ok(s) = std::process::Command::new(player)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            if s.success() {
                let _ = std::fs::remove_file(tmp);
                return Ok(());
            }
        }
    }
    let _ = std::fs::remove_file(tmp);
    Err(anyhow::anyhow!("No audio player found"))
}

fn write_wav(path: &str, samples: &[f32], sample_rate: u32) -> Result<()> {
    use std::io::Write;
    let n = samples.len() as u32;
    let data_size = n * 2;
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
    for &s in samples {
        f.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    Ok(())
}
