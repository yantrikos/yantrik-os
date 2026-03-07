//! Audio conversion utilities — shell out to ffmpeg and edge-tts/espeak-ng.
//!
//! Used by Telegram voice pipeline (Jarvis mode) to convert between
//! OGG/Opus (Telegram's voice format) and raw PCM (Whisper's input format),
//! and to synthesize speech from text.
//!
//! TTS priority: edge-tts (Microsoft neural voices) → espeak-ng (fallback).

/// Convert OGG/Opus file to 16 kHz mono f32 PCM samples (for Whisper STT).
pub fn ogg_to_pcm_f32(ogg_path: &str) -> Result<Vec<f32>, String> {
    let output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(ogg_path)
        .arg("-f").arg("f32le")
        .arg("-ar").arg("16000")
        .arg("-ac").arg("1")
        .arg("pipe:1")
        .arg("-loglevel").arg("error")
        .arg("-y")
        .output()
        .map_err(|e| format!("ffmpeg not found or failed to run: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg decode failed: {stderr}"));
    }

    let bytes = &output.stdout;
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "ffmpeg output length {} is not a multiple of 4 (f32)",
            bytes.len()
        ));
    }

    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();

    Ok(samples)
}

/// Convert text to OGG/Opus voice message.
///
/// Uses edge-tts (Microsoft neural voices) if available, falls back to espeak-ng.
/// `voice` is the edge-tts voice name (e.g. "en-US-GuyNeural").
/// `rate` and `pitch` are espeak-ng parameters (only used for fallback).
pub fn text_to_ogg(text: &str, out_path: &str, rate: u32, pitch: u32) -> Result<(), String> {
    // Try edge-tts first (high quality neural TTS)
    if has_edge_tts() {
        return text_to_ogg_edge(text, out_path);
    }

    // Fallback to espeak-ng (robotic but works offline)
    text_to_ogg_espeak(text, out_path, rate, pitch)
}

/// High-quality TTS via edge-tts (Microsoft neural voices).
fn text_to_ogg_edge(text: &str, out_path: &str) -> Result<(), String> {
    // edge-tts outputs MP3, we convert to OGG/Opus via ffmpeg
    let mp3_path = format!("{}.mp3", out_path);

    let edge_output = std::process::Command::new("edge-tts")
        .arg("--text").arg(text)
        .arg("--voice").arg("en-US-GuyNeural")
        .arg("--rate").arg("+10%")
        .arg("--write-media").arg(&mp3_path)
        .output()
        .map_err(|e| format!("edge-tts failed: {e}"))?;

    if !edge_output.status.success() {
        let stderr = String::from_utf8_lossy(&edge_output.stderr);
        // Clean up and fall back
        let _ = std::fs::remove_file(&mp3_path);
        return Err(format!("edge-tts failed: {stderr}"));
    }

    // Convert MP3 → OGG/Opus
    let ffmpeg_output = std::process::Command::new("ffmpeg")
        .arg("-i").arg(&mp3_path)
        .arg("-c:a").arg("libopus")
        .arg("-b:a").arg("48k")
        .arg("-loglevel").arg("error")
        .arg("-y")
        .arg(out_path)
        .output()
        .map_err(|e| format!("ffmpeg encode failed: {e}"))?;

    // Clean up MP3
    let _ = std::fs::remove_file(&mp3_path);

    if !ffmpeg_output.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
        return Err(format!("ffmpeg MP3→OGG failed: {stderr}"));
    }

    Ok(())
}

/// Fallback TTS via espeak-ng (robotic but offline).
fn text_to_ogg_espeak(text: &str, out_path: &str, rate: u32, pitch: u32) -> Result<(), String> {
    let espeak = std::process::Command::new("espeak-ng")
        .arg("--stdout")
        .arg("-s").arg(rate.to_string())
        .arg("-p").arg(pitch.to_string())
        .arg(text)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("espeak-ng not found or failed to run: {e}"))?;

    let espeak_stdout = espeak.stdout.ok_or("Failed to capture espeak-ng stdout")?;

    let ffmpeg_output = std::process::Command::new("ffmpeg")
        .arg("-i").arg("pipe:0")
        .arg("-c:a").arg("libopus")
        .arg("-b:a").arg("48k")
        .arg("-loglevel").arg("error")
        .arg("-y")
        .arg(out_path)
        .stdin(espeak_stdout)
        .output()
        .map_err(|e| format!("ffmpeg encode failed to run: {e}"))?;

    if !ffmpeg_output.status.success() {
        let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
        return Err(format!("ffmpeg encode failed: {stderr}"));
    }

    Ok(())
}

/// Check if edge-tts is installed.
fn has_edge_tts() -> bool {
    std::process::Command::new("edge-tts")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if ffmpeg and at least one TTS engine are available.
pub fn check_dependencies() -> Result<(), String> {
    let ffmpeg = std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if ffmpeg.is_err() || !ffmpeg.unwrap().success() {
        return Err("ffmpeg is not installed or not in PATH".into());
    }

    // Check for at least one TTS engine
    let has_edge = has_edge_tts();
    let has_espeak = std::process::Command::new("espeak-ng")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !has_edge && !has_espeak {
        return Err("No TTS engine found (install edge-tts or espeak-ng)".into());
    }

    if has_edge {
        tracing::info!("TTS: using edge-tts (Microsoft neural voices)");
    } else {
        tracing::info!("TTS: using espeak-ng (fallback)");
    }

    Ok(())
}
