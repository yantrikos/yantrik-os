//! The machine's volume: PipeWire's default output, read and set through `wpctl`.
//!
//! The image runs PipeWire with WirePlumber. The shell used to move the raw ALSA mixer
//! instead, which is a different control from the one the volume keys and every other
//! app move, so the slider and the speaker could disagree. Everything here goes through the one
//! the session's audio server owns.
//!
//! Commands run with argument vectors, never through a shell. A machine with no `wpctl` (or no
//! audio server) answers `None`/`Err`: there is no guessed value to fall back to.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crossbeam_channel::Sender;

use crate::events::SystemEvent;

const DEFAULT_SINK: &str = "@DEFAULT_AUDIO_SINK@";
const DEFAULT_SOURCE: &str = "@DEFAULT_AUDIO_SOURCE@";

/// What the default output is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioState {
    /// 0..=100. A sink turned up past 100% (wpctl allows it) reads as 100: the slider's range
    /// ends there, and the shell never sets more.
    pub volume_pct: u8,
    pub muted: bool,
}

/// Parse the output of `wpctl get-volume @DEFAULT_AUDIO_SINK@`, which looks like
/// `Volume: 0.45` or `Volume: 0.45 [MUTED]`. Anything else is not a reading.
pub fn parse_get_volume(out: &str) -> Option<AudioState> {
    let rest = out.trim().strip_prefix("Volume:")?;
    let mut parts = rest.split_whitespace();
    let fraction: f64 = parts.next()?.parse().ok()?;
    if !fraction.is_finite() || fraction < 0.0 {
        return None;
    }
    let muted = parts.any(|p| p == "[MUTED]");
    let volume_pct = (fraction * 100.0).round().min(100.0) as u8;
    Some(AudioState { volume_pct, muted })
}

/// The current volume and mute of the default output, or `None` when there is no audio server
/// to ask.
pub fn read() -> Option<AudioState> {
    let out = Command::new("wpctl")
        .args(["get-volume", DEFAULT_SINK])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_get_volume(&String::from_utf8_lossy(&out.stdout))
}

/// The default output's name as the audio server gives it (`node.description`, the words
/// PipeWire and every mixer show: "Built-in Audio Analog Stereo"), out of `wpctl inspect`.
/// `None` when the line is not there or says nothing: no name is better than a guessed one.
pub fn parse_output_name(inspect: &str) -> Option<String> {
    inspect.lines().find_map(|line| {
        let line = line.trim().trim_start_matches('*').trim_start();
        let value = line.strip_prefix("node.description")?.trim_start().strip_prefix('=')?.trim();
        let name = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value).trim();
        (!name.is_empty()).then(|| name.to_string())
    })
}

/// The default output's name, or `None` when there is no audio server to ask or it names none.
pub fn read_output_name() -> Option<String> {
    let out = Command::new("wpctl")
        .args(["inspect", DEFAULT_SINK])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_output_name(&String::from_utf8_lossy(&out.stdout))
}

/// The argument vector that sets the volume. `-l 1.0` is wpctl's own ceiling, so even a value
/// that slipped past the clamp could not push the sink over 100%.
fn set_volume_args(pct: u8) -> Vec<String> {
    vec![
        "set-volume".into(),
        "-l".into(),
        "1.0".into(),
        DEFAULT_SINK.into(),
        format!("{}%", pct.min(100)),
    ]
}

/// The argument vector for moving the volume by `step` percent in ONE wpctl call. wpctl applies
/// the step to the level the server holds at that instant, so two key presses that overlap each
/// move it twice; reading the level first and writing a target lost one of them (review of the
/// on-screen display). `-l 1.0` is the ceiling on the way up, and wpctl stops at 0 on the way
/// down by itself.
fn step_volume_args(step: i8) -> Vec<String> {
    let amount = format!("{}%{}", step.unsigned_abs(), if step < 0 { "-" } else { "+" });
    let mut args: Vec<String> = vec!["set-volume".into()];
    if step > 0 {
        args.extend(["-l".to_string(), "1.0".to_string()]);
    }
    args.extend([DEFAULT_SINK.to_string(), amount]);
    args
}

fn run_wpctl(args: &[String]) -> Result<(), String> {
    let out = Command::new("wpctl")
        .args(args)
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("wpctl could not be run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("wpctl failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Set the default output's volume, clamped to 0..=100.
pub fn set_volume(pct: u8) -> Result<(), String> {
    run_wpctl(&set_volume_args(pct))
}

/// Move the volume of the default output by `step` percent, atomically (see
/// [`step_volume_args`]).
pub fn step_volume(step: i8) -> Result<(), String> {
    run_wpctl(&step_volume_args(step))
}

/// Mute or unmute the default output.
pub fn set_mute(muted: bool) -> Result<(), String> {
    run_wpctl(&set_mute_args(DEFAULT_SINK, muted))
}

/// Whether the default input (the microphone) is muted, or `None` when there is no audio server
/// or no input to ask. Same `wpctl get-volume` line as the output, so the same parser reads it;
/// only the mute is kept, because the shell has no microphone level to show.
pub fn read_mic_muted() -> Option<bool> {
    let out = Command::new("wpctl")
        .args(["get-volume", DEFAULT_SOURCE])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_get_volume(&String::from_utf8_lossy(&out.stdout)).map(|s| s.muted)
}

/// Mute or unmute the default input.
pub fn set_mic_mute(muted: bool) -> Result<(), String> {
    run_wpctl(&set_mute_args(DEFAULT_SOURCE, muted))
}

fn set_mute_args(target: &str, muted: bool) -> Vec<String> {
    vec!["set-mute".into(), target.into(), if muted { "1" } else { "0" }.into()]
}

/// Whether one line of `pactl subscribe` is about an output: a sink changing (volume, mute) or
/// the server picking another default. Input, stream and card lines are not the volume.
fn is_output_event(line: &str) -> bool {
    let l = line.trim();
    l.starts_with("Event 'change' on sink #") || l.starts_with("Event 'change' on server")
}

/// How long the lines must stop for before the level is read. A slider drag or a held key
/// makes dozens of events a second; one reading after they settle is the value that matters.
const SETTLE: Duration = Duration::from_millis(120);

/// How long to wait before asking the audio server again after it went away.
const RETRY: Duration = Duration::from_secs(5);

/// Watch the audio server and say `AudioChanged` when the default output's volume or mute
/// changes, whoever changed it: the keys, another app, `wpctl` in a terminal.
///
/// Stops at once, logging once, when `pactl` is not installed. When the server goes away
/// (a restart of PipeWire) it asks again every few seconds, without logging each time.
pub fn run_audio_watcher(tx: Sender<SystemEvent>) {
    let mut last = read();
    let mut warned = false;
    loop {
        let mut child = match Command::new("pactl")
            .arg("subscribe")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!("pactl is not installed; the shell will not follow volume changes made elsewhere");
                return;
            }
            Err(e) => {
                if !warned {
                    tracing::warn!(error = %e, "could not start `pactl subscribe`; retrying quietly");
                    warned = true;
                }
                std::thread::sleep(RETRY);
                continue;
            }
        };
        let Some(stdout) = child.stdout.take() else { return };
        tracing::info!("Audio watcher started (pactl subscribe)");

        // A reader thread, so the settling wait below can time out. It ends when pactl does.
        let (line_tx, line_rx) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if is_output_event(&line) && line_tx.send(()).is_err() {
                    break;
                }
            }
        });

        // `Err` from the blocking recv means the reader finished: pactl exited.
        while line_rx.recv().is_ok() {
            while line_rx.recv_timeout(SETTLE).is_ok() {}
            let now = read();
            if now != last {
                last = now;
                if let Some(AudioState { volume_pct, muted }) = now {
                    let _ = tx.try_send(SystemEvent::AudioChanged { volume_pct, muted });
                }
            }
        }
        let _ = child.wait();
        std::thread::sleep(RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_reading() {
        assert_eq!(parse_get_volume("Volume: 0.45\n"), Some(AudioState { volume_pct: 45, muted: false }));
        assert_eq!(parse_get_volume("Volume: 1.00\n"), Some(AudioState { volume_pct: 100, muted: false }));
        assert_eq!(parse_get_volume("Volume: 0.00\n"), Some(AudioState { volume_pct: 0, muted: false }));
    }

    #[test]
    fn a_muted_sink_keeps_its_level() {
        assert_eq!(parse_get_volume("Volume: 0.45 [MUTED]\n"), Some(AudioState { volume_pct: 45, muted: true }));
    }

    #[test]
    fn a_sink_turned_up_past_full_reads_as_full() {
        assert_eq!(parse_get_volume("Volume: 1.20\n"), Some(AudioState { volume_pct: 100, muted: false }));
        assert_eq!(parse_get_volume("Volume: 1.53 [MUTED]\n"), Some(AudioState { volume_pct: 100, muted: true }));
    }

    #[test]
    fn rounding_follows_the_nearest_percent() {
        assert_eq!(parse_get_volume("Volume: 0.336\n").map(|s| s.volume_pct), Some(34));
        assert_eq!(parse_get_volume("Volume: 0.304\n").map(|s| s.volume_pct), Some(30));
    }

    #[test]
    fn anything_else_is_not_a_reading() {
        for bad in [
            "",
            "\n",
            "Volume:",
            "Volume: loud",
            "Volume: -0.5",
            "Volume: NaN",
            "Volume: inf",
            "Could not connect to PipeWire",
            "wpctl: command not found",
            "0.45",
        ] {
            assert_eq!(parse_get_volume(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_set_command_is_an_argument_vector_capped_at_full() {
        assert_eq!(
            set_volume_args(45),
            ["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", "45%"]
        );
        assert_eq!(set_volume_args(250).last().map(String::as_str), Some("100%"));
    }

    #[test]
    fn the_microphone_is_muted_with_its_own_target_not_the_speakers() {
        assert_eq!(set_mute_args(DEFAULT_SOURCE, true), ["set-mute", "@DEFAULT_AUDIO_SOURCE@", "1"]);
        assert_eq!(set_mute_args(DEFAULT_SINK, false), ["set-mute", "@DEFAULT_AUDIO_SINK@", "0"]);
    }

    #[test]
    fn a_microphone_reading_is_the_same_line_as_a_speakers() {
        assert_eq!(parse_get_volume("Volume: 1.00 [MUTED]\n").map(|s| s.muted), Some(true));
        assert_eq!(parse_get_volume("Volume: 1.00\n").map(|s| s.muted), Some(false));
    }

    #[test]
    fn only_output_lines_wake_the_watcher() {
        assert!(is_output_event("Event 'change' on sink #53"));
        assert!(is_output_event("Event 'change' on server #0"));
        assert!(!is_output_event("Event 'change' on source #54"));
        assert!(!is_output_event("Event 'new' on sink-input #120"));
        assert!(!is_output_event("Event 'remove' on sink-input #120"));
        assert!(!is_output_event("Event 'change' on card #2"));
    }

    #[test]
    fn the_output_is_named_by_its_description_or_not_at_all() {
        let inspect = "id 52, type PipeWire:Interface:Node
    alsa.card = \"0\"
  * node.description = \"Built-in Audio Analog Stereo\"
  * node.name = \"alsa_output.pci-0000_00_1f.3.analog-stereo\"
";
        assert_eq!(parse_output_name(inspect).as_deref(), Some("Built-in Audio Analog Stereo"));
        assert_eq!(parse_output_name("    node.description = \"HDMI\"").as_deref(), Some("HDMI"));
        for nothing in ["", "id 52, type PipeWire:Interface:Node
  * node.name = \"alsa_output.x\"", "  * node.description = \"\"", "  * node.descriptionx = \"A\""] {
            assert_eq!(parse_output_name(nothing), None, "{nothing:?}");
        }
    }

    #[test]
    fn a_step_is_one_relative_wpctl_call_never_a_read_then_a_write() {
        assert_eq!(
            step_volume_args(5),
            ["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", "5%+"]
        );
        assert_eq!(step_volume_args(-5), ["set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]);
    }
}
