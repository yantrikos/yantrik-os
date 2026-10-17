//! Screen recording: wf-recorder in a transient user service of its own.
//!
//! Each recording is `systemd-run --user --unit=yantrik-capture-rec-<n> --collect`, so it can be
//! stopped and accounted for by name, its CPU and memory are its own cgroup's, and it is the
//! person's work at normal weight rather than a child of the shell that dies with it. The unit's
//! stop signal is SIGINT, which is wf-recorder's "finish the file": a plain SIGTERM or a kill
//! leaves an MP4 with no index, which no player opens. So every way a recording ends — the red
//! dot, Super+Shift+R, a mind's `record_stop`, the screen locking, logging out, the disk filling
//! — is `systemctl --user stop`, which sends SIGINT and waits for the file to be finished.
//!
//! One recording at a time, whoever started it. The red dot in the status bar shows for every
//! one of them; nothing a caller passes can hide it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use super::disk;
use super::encoder::{self, Encoder};
use super::grab::{self, Display};
use super::names::{self, Kind};
use super::Who;

/// How long a clean stop may take: wf-recorder flushes the encoder and writes the index. A few
/// hundred milliseconds normally; the unit is given this long before systemd kills it.
const STOP_TIMEOUT_SECS: u64 = 15;

/// The longest a mind's recording runs (`RuntimeMaxSec`): it was the person's work at the person's
/// weight with no end, so once "allow for this session" covered `record_start`, a mind could
/// leave one running until the disk guard tripped (security review of #828, finding 5). systemd
/// stops it with the unit's SIGINT, which finishes the file.
pub const MIND_MAX_SECS: u64 = 30 * 60;

/// A mind's recording's CPU weight, a fifth of the default 100: an agent's work yields to the
/// person's. There is no agents' slice yet to put it in, so the weight is on the unit itself.
pub const MIND_CPU_WEIGHT: u32 = 20;

/// How often the watch looks: the elapsed time on the bar, and whether the unit still runs.
const TICK: Duration = Duration::from_secs(1);

/// How often the disk is asked, in ticks.
const DISK_EVERY: u64 = 5;

/// A recording that ended this soon after it started, on the GPU, is the encoder refusing (a
/// driver that lists H.264 encode and cannot do it at this size): it is started again on the CPU,
/// once.
const GPU_GRACE: Duration = Duration::from_secs(4);

/// What is being recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The person's desktop, or a rectangle of it in grim's geometry. Only the person starts
    /// one: no action a mind can call builds this.
    Desktop { region: Option<String> },
    /// Mind View, the minds' own display.
    MindView,
}

impl Target {
    fn display(&self) -> Display {
        match self {
            Target::Desktop { .. } => Display::Desktop,
            Target::MindView => Display::MindView,
        }
    }

    /// What `describe shell` says it records.
    pub fn word(&self) -> &'static str {
        match self {
            Target::Desktop { region: None } => "screen",
            Target::Desktop { region: Some(_) } => "region",
            Target::MindView => "mind_view",
        }
    }
}

/// Sound in the recording. The person's choice on the bar; a mind's recording has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audio {
    None,
    /// What the speakers play: the default sink's monitor.
    System,
    /// The default microphone.
    Mic,
}

/// Why a recording ended, which the notification says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    Person,
    Mind(String),
    Locked,
    LoggingOut,
    DiskLow(u64),
    /// Found running when the shell started, from a shell that ended without stopping it.
    Leftover,
    /// wf-recorder ended by itself.
    Ended,
    /// A mind's recording reached [`MIND_MAX_SECS`].
    MindLimit,
    /// The person started a recording of their own over a mind's.
    PersonStarted,
}

impl Why {
    /// The sentence the finished-recording notification adds, if any.
    pub fn sentence(&self) -> Option<String> {
        match self {
            Why::Person => None,
            Why::Mind(name) => Some(format!("{name} stopped it.")),
            Why::Locked => Some("It stopped when the screen locked.".into()),
            Why::LoggingOut => Some("It stopped when you logged out.".into()),
            Why::DiskLow(free) => Some(format!(
                "It stopped because the disk has {} free, under the 1 GB a recording keeps clear.",
                disk::words(*free)
            )),
            Why::Leftover => Some("It was still running when the desktop restarted, so it was stopped.".into()),
            Why::Ended => Some("The recorder ended by itself.".into()),
            Why::MindLimit => Some(format!("It stopped after {} minutes, the longest a mind may record.", MIND_MAX_SECS / 60)),
            Why::PersonStarted => Some("It stopped because you started a recording of your own.".into()),
        }
    }
}

struct Running {
    unit: String,
    path: PathBuf,
    started: Instant,
    wall: DateTime<Local>,
    target: Target,
    audio: Audio,
    who: Who,
    encoder: Encoder,
    /// Set once a stop has been asked for, so the watch does not report the end as unexpected.
    stopping: bool,
    /// The GPU start was already given its one retry on the CPU.
    retried: bool,
}

static STATE: Mutex<Option<Running>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(0);

fn state() -> std::sync::MutexGuard<'static, Option<Running>> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// The next unit name. Numbered from the clock at first use, so a shell started again does not
/// reuse the name of a unit the last one left behind.
fn next_unit() -> String {
    let _ = NEXT.compare_exchange(
        0,
        chrono::Utc::now().timestamp().max(1) as u64,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    format!("yantrik-capture-rec-{}", NEXT.fetch_add(1, Ordering::AcqRel))
}

/// The whole command line that starts a recording. Pure, so the tests read exactly what runs.
pub fn command(unit: &str, wayland: &str, target: &Target, audio: Audio, enc: &Encoder, path: &Path, who: &Who) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "systemd-run".into(),
        "--user".into(),
        format!("--unit={unit}"),
        "--collect".into(),
        // What System Monitor and `systemctl --user status` call it.
        "--description=Screen recording".into(),
        // wf-recorder's "finish the file and exit", for every stop systemd makes.
        "--property=KillSignal=SIGINT".into(),
        format!("--property=TimeoutStopSec={STOP_TIMEOUT_SECS}"),
        // The video is the person's alone: wf-recorder makes the file itself, at the umask, and
        // the default left it 0644 (security review of #828, finding 3). The folder is 0700 and
        // the name fresh (names::fresh_path).
        "--property=UMask=0077".into(),
        // The display to record. WAYLAND_SOCKET is not passed: the unit is started by the user
        // manager, which has the session's environment, not the shell's handed socket.
        format!("--setenv=WAYLAND_DISPLAY={wayland}"),
    ];
    if let Who::Mind { .. } = who {
        argv.push(format!("--property=RuntimeMaxSec={MIND_MAX_SECS}"));
        argv.push(format!("--property=CPUWeight={MIND_CPU_WEIGHT}"));
    }
    argv.extend(["--".to_string(), "wf-recorder".to_string()]);
    argv.extend(enc.args());
    if let Target::Desktop { region: Some(g) } = target {
        argv.extend(["-g".to_string(), g.clone()]);
    }
    match audio {
        Audio::None => {}
        // PulseAudio's names for the defaults, which PipeWire's pulse server answers to.
        Audio::System => argv.push("--audio=@DEFAULT_MONITOR@".into()),
        Audio::Mic => argv.push("--audio=@DEFAULT_SOURCE@".into()),
    }
    argv.extend(["-f".to_string(), path.to_string_lossy().into_owned()]);
    argv
}

fn run(argv: &[String], limit: Duration, what: &str) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    grab::run_bounded(cmd, limit, what)
}

/// Whether one is running.
pub fn running() -> bool {
    state().is_some()
}

/// Whether the person's own recording is running: what the bar's button stops. A mind's is
/// stopped by the red dot, or by the person starting their own over it.
pub fn running_for_person() -> bool {
    state().as_ref().is_some_and(|r| r.who == Who::Person)
}

/// Whether `starting` may start while `running` runs: `Ok(true)` when what runs must be stopped
/// first. The person's recording outranks a mind's, which once kept the person from recording
/// their own screen until they found the dot (security review of #828, finding 5); nothing else
/// starts over a running recording.
pub fn makes_way(running: Option<&Who>, starting: &Who) -> Result<bool, String> {
    match (running, starting) {
        (None, _) => Ok(false),
        (Some(Who::Mind { .. }), Who::Person) => Ok(true),
        (Some(_), _) => Err("a recording is already running; stop it first (the red dot in the status bar)".into()),
    }
}

/// Start one. Blocking (systemd-run, vainfo): call it on a worker.
pub fn start(target: Target, audio: Audio, who: Who) -> Result<PathBuf, String> {
    let running_now = state().as_ref().map(|r| r.who.clone());
    if makes_way(running_now.as_ref(), &who)? {
        let _ = stop(Why::PersonStarted, None)?;
    }
    if running() {
        return Err("a recording is already running; stop it first (the red dot in the status bar)".into());
    }
    let wayland = target.display().wayland()?;
    let home = names::home()?;
    let dir = names::folder(Kind::Recording, &home)?;
    let free = disk::free_bytes(&dir);
    if !disk::room(free) {
        return Err(format!(
            "the disk has {} free, and a recording needs at least 1 GB clear, so none was started",
            disk::words(free.unwrap_or(0))
        ));
    }
    let wall = Local::now();
    let path = names::fresh_path(Kind::Recording, &home, &wall)?;
    let enc = encoder::probe();
    let unit = next_unit();
    launch(&unit, &wayland, &target, audio, &enc, &path, &who)?;
    tracing::info!(unit, path = %path.display(), encoder = enc.name(), records = target.word(), "Recording started");
    *state() = Some(Running {
        unit: unit.clone(),
        path: path.clone(),
        started: Instant::now(),
        wall,
        target,
        audio,
        who,
        encoder: enc,
        stopping: false,
        retried: false,
    });
    super::show_recording(true, "0:00");
    std::thread::Builder::new()
        .name("yantrik-capture-watch".into())
        .spawn(move || watch(unit))
        .map_err(|e| format!("could not watch the recording: {e}"))?;
    Ok(path)
}

fn launch(unit: &str, wayland: &str, target: &Target, audio: Audio, enc: &Encoder, path: &Path, who: &Who) -> Result<(), String> {
    let argv = command(unit, wayland, target, audio, enc, path, who);
    run(&argv, Duration::from_secs(10), "systemd-run").map(|_| ())
}

/// Stop the running recording, cleanly, and say where it went. Blocking for as long as the file
/// takes to finish (up to [`STOP_TIMEOUT_SECS`]): call it on a worker.
///
/// `only_by`: a mind may stop only a recording that mind started; `None` stops any (the person,
/// the lock, logging out, the disk).
pub fn stop(why: Why, only_by: Option<&Who>) -> Result<Option<PathBuf>, String> {
    let unit = {
        let mut guard = state();
        let Some(rec) = guard.as_mut() else { return Ok(None) };
        if let Some(who) = only_by {
            if !rec.who.started_by(who) {
                return Err("the recording running now was not started by you, so it was left running; \
                            the person stops it with the red dot"
                    .into());
            }
        }
        if rec.stopping {
            return Ok(Some(rec.path.clone()));
        }
        rec.stopping = true;
        rec.unit.clone()
    };
    let argv: Vec<String> = ["systemctl", "--user", "stop", &unit].iter().map(|s| s.to_string()).collect();
    let stopped = run(&argv, Duration::from_secs(STOP_TIMEOUT_SECS + 10), "systemctl --user stop");
    let rec = state().take();
    super::show_recording(false, "");
    let Some(rec) = rec else { return Ok(None) };
    if let Err(e) = stopped {
        tracing::warn!(unit, error = %e, "The recording's unit did not stop cleanly");
    }
    finished(&rec, &why);
    Ok(Some(rec.path))
}

/// Tell the person a recording ended, with its file's buttons, or that nothing usable was kept.
fn finished(rec: &Running, why: &Why) {
    let size = std::fs::metadata(&rec.path).map(|m| m.len()).unwrap_or(0);
    let length = rec.started.elapsed();
    tracing::info!(unit = rec.unit, path = %rec.path.display(), bytes = size, secs = length.as_secs(), ?why, "Recording finished");
    super::notice::recording_finished(&rec.path, size, length, why);
}

/// Stop it at lock, from the lock callback (UI thread): the stop itself waits for the file, so
/// it runs on a worker. Nothing is captured while the screen is locked, and a recording is no
/// exception.
pub fn stop_soon(why: Why) {
    if !running() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("yantrik-capture-stop".into())
        .spawn(move || {
            let _ = stop(why, None);
        });
}

/// What the watch does about a reading. Pure: the tests hold the lock and disk rules here.
pub fn verdict(locked: bool, free: Option<u64>) -> Option<Why> {
    if locked {
        return Some(Why::Locked);
    }
    match free {
        Some(f) if !disk::room(Some(f)) => Some(Why::DiskLow(f)),
        _ => None,
    }
}

/// "0:42", "12:03", "1:02:03".
pub fn elapsed_words(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn is_active(unit: &str) -> bool {
    let mut cmd = Command::new("systemctl");
    cmd.args(["--user", "is-active", "--quiet", unit]);
    grab::run_bounded(cmd, Duration::from_secs(3), "systemctl --user is-active").is_ok()
}

/// The watch: one thread per recording, gone when it is. Updates the bar's elapsed time, stops
/// for the lock and the disk, and notices a recorder that ended by itself.
fn watch(unit: String) {
    let mut tick = 0u64;
    loop {
        std::thread::sleep(TICK);
        tick += 1;
        let (started, dir, encoder, retried, by_mind) = {
            let guard = state();
            match guard.as_ref() {
                Some(r) if r.unit == unit && !r.stopping => (
                    r.started,
                    r.path.parent().map(Path::to_path_buf),
                    r.encoder.clone(),
                    r.retried,
                    matches!(r.who, Who::Mind { .. }),
                ),
                _ => return,
            }
        };
        super::show_recording(true, &elapsed_words(started.elapsed()));
        let free = if tick.is_multiple_of(DISK_EVERY) { dir.as_deref().and_then(disk::free_bytes) } else { None };
        if let Some(why) = verdict(super::locked_now(), free) {
            let _ = stop(why, None);
            return;
        }
        if tick.is_multiple_of(2) && !is_active(&unit) {
            let gpu_refused = matches!(encoder, Encoder::Vaapi(_)) && !retried && started.elapsed() < GPU_GRACE;
            if gpu_refused && retry_on_cpu(&unit) {
                continue;
            }
            let rec = {
                let mut guard = state();
                match guard.as_ref() {
                    Some(r) if r.unit == unit && !r.stopping => guard.take(),
                    _ => None,
                }
            };
            if let Some(rec) = rec {
                super::show_recording(false, "");
                // systemd ended a mind's at its limit; a few seconds' slack for the ticks.
                let limit = Duration::from_secs(MIND_MAX_SECS.saturating_sub(5));
                let why = if by_mind && started.elapsed() >= limit { Why::MindLimit } else { Why::Ended };
                finished(&rec, &why);
            }
            return;
        }
    }
}

/// Start the same recording again with the CPU encoder, under the same unit name and file.
fn retry_on_cpu(unit: &str) -> bool {
    let (wayland, target, audio, path, who) = {
        let guard = state();
        let Some(r) = guard.as_ref().filter(|r| r.unit == unit) else { return false };
        let Ok(wayland) = r.target.display().wayland() else { return false };
        (wayland, r.target.clone(), r.audio, r.path.clone(), r.who.clone())
    };
    tracing::warn!(unit, "The GPU encoder ended the recording at once; starting it again on the CPU");
    // What the refused start left is not a recording; wf-recorder would ask before overwriting it.
    let _ = std::fs::remove_file(&path);
    if launch(unit, &wayland, &target, audio, &Encoder::X264, &path, &who).is_err() {
        return false;
    }
    if let Some(r) = state().as_mut().filter(|r| r.unit == unit) {
        r.encoder = Encoder::X264;
        r.retried = true;
        r.started = Instant::now();
    }
    true
}

/// Recordings a shell before this one left running (it crashed, or was restarted mid-recording)
/// have no dot on any bar, so nobody would know to stop them. They are stopped, cleanly, and the
/// person told. Blocking: a worker at startup.
pub fn stop_leftovers() {
    let mut cmd = Command::new("systemctl");
    cmd.args(["--user", "list-units", "--plain", "--no-legend", "--state=active", "yantrik-capture-rec-*"]);
    let Ok(out) = grab::run_bounded(cmd, Duration::from_secs(5), "systemctl --user list-units") else { return };
    for line in String::from_utf8_lossy(&out).lines() {
        let Some(unit) = line.split_whitespace().next().filter(|u| u.starts_with("yantrik-capture-rec-")) else {
            continue;
        };
        let mut stop = Command::new("systemctl");
        stop.args(["--user", "stop", unit]);
        let _ = grab::run_bounded(stop, Duration::from_secs(STOP_TIMEOUT_SECS + 10), "systemctl --user stop");
        tracing::info!(unit, "Stopped a recording a previous shell left running");
        super::notice::leftover_stopped();
    }
}

/// `describe shell`'s `capture`: whether a recording runs, since when, what it records, who
/// started it and where it will be saved.
pub fn for_describe() -> serde_json::Value {
    let guard = state();
    match guard.as_ref() {
        None => serde_json::json!({ "recording": false }),
        Some(r) => serde_json::json!({
            "recording": true,
            "since": r.wall.to_rfc3339(),
            "elapsed": elapsed_words(r.started.elapsed()),
            "records": r.target.word(),
            "started_by": r.who.words(),
            "saving_to": r.path.to_string_lossy(),
            "audio": match r.audio { Audio::None => "none", Audio::System => "system", Audio::Mic => "microphone" },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_runs_in_a_transient_unit_that_stops_with_sigint() {
        let path = Path::new("/home/p/Videos/Recordings/Recording 2026-10-15 at 14.32.07.mp4");
        let argv = command("yantrik-capture-rec-7", "wayland-0", &Target::Desktop { region: None }, Audio::None, &Encoder::X264, path, &Who::Person);
        assert_eq!(&argv[..4], ["systemd-run", "--user", "--unit=yantrik-capture-rec-7", "--collect"]);
        assert!(argv.contains(&"--property=KillSignal=SIGINT".to_string()), "{argv:?}");
        assert!(argv.contains(&"--setenv=WAYLAND_DISPLAY=wayland-0".to_string()));
        // The file wf-recorder makes is the person's alone (security review of #828, finding 3).
        assert!(argv[..argv.iter().position(|a| a == "--").unwrap()].contains(&"--property=UMask=0077".to_string()), "{argv:?}");
        let rec = argv.iter().position(|a| a == "wf-recorder").expect("runs wf-recorder");
        assert_eq!(argv[rec - 1], "--", "everything after -- is the recorder's");
        assert_eq!(argv.last().unwrap(), &path.to_string_lossy());
        assert!(!argv.iter().any(|a| a.starts_with("--audio")), "no audio unless chosen");
        assert!(!argv.iter().any(|a| a == "-g"), "the whole screen has no geometry");
        assert!(!argv.iter().any(|a| a.starts_with("--property=RuntimeMaxSec")), "the person's has no end but theirs");
    }

    /// Security review of #828, finding 5: a mind's recording ends by itself, yields the CPU, and
    /// never keeps the person from recording their own screen.
    #[test]
    fn a_minds_recording_is_capped_light_and_gives_way_to_the_persons() {
        let mind = Who::Mind { name: "pi".into(), id: Some("agent:pi:main".into()) };
        let argv = command("u", "wayland-1", &Target::MindView, Audio::None, &Encoder::X264, Path::new("/x.mp4"), &mind);
        let unit = &argv[..argv.iter().position(|a| a == "--").unwrap()];
        assert!(unit.contains(&format!("--property=RuntimeMaxSec={MIND_MAX_SECS}")), "{argv:?}");
        assert!(unit.contains(&format!("--property=CPUWeight={MIND_CPU_WEIGHT}")), "{argv:?}");
        assert_eq!(MIND_MAX_SECS, 1800);
        const { assert!(MIND_CPU_WEIGHT < 100, "under the default weight") };

        assert_eq!(makes_way(None, &Who::Person), Ok(false));
        assert_eq!(makes_way(None, &mind), Ok(false));
        assert_eq!(makes_way(Some(&mind), &Who::Person), Ok(true), "the person's Start stops a mind's");
        assert!(makes_way(Some(&Who::Person), &mind).is_err(), "a mind never stops the person's");
        assert!(makes_way(Some(&mind), &mind).is_err(), "nor another mind's");
        assert!(makes_way(Some(&Who::Person), &Who::Person).is_err());
        assert!(Why::MindLimit.sentence().unwrap().contains("30 minutes"));
        assert!(Why::PersonStarted.sentence().unwrap().contains("recording of your own"));
    }

    #[test]
    fn a_region_and_sound_reach_the_recorder() {
        let region = Target::Desktop { region: Some("10,20 640x400".into()) };
        let argv = command("u", "wayland-0", &region, Audio::System, &Encoder::X264, Path::new("/x.mp4"), &Who::Person);
        assert!(argv.windows(2).any(|w| w == ["-g", "10,20 640x400"]), "{argv:?}");
        assert!(argv.contains(&"--audio=@DEFAULT_MONITOR@".to_string()));
        let argv = command("u", "wayland-0", &region, Audio::Mic, &Encoder::X264, Path::new("/x.mp4"), &Who::Person);
        assert!(argv.contains(&"--audio=@DEFAULT_SOURCE@".to_string()));
        assert_eq!(region.word(), "region");
        assert_eq!(Target::MindView.word(), "mind_view");
    }

    #[test]
    fn the_lock_stops_a_recording_and_so_does_a_full_disk() {
        assert_eq!(verdict(true, None), Some(Why::Locked));
        assert_eq!(verdict(true, Some(50 << 30)), Some(Why::Locked), "locked wins whatever the disk says");
        assert_eq!(verdict(false, Some(disk::MIN_FREE - 1)), Some(Why::DiskLow(disk::MIN_FREE - 1)));
        assert_eq!(verdict(false, Some(50 << 30)), None);
        assert_eq!(verdict(false, None), None, "a tick that did not read the disk changes nothing");
        assert!(Why::Locked.sentence().unwrap().contains("screen locked"));
        assert!(Why::DiskLow(512 << 20).sentence().unwrap().contains("0.5 GB free"));
    }

    #[test]
    fn the_lock_callback_stops_the_recording_before_the_screen_goes_dark() {
        // The watch would catch the lock within a second; the lock callback does not wait for it.
        let callbacks = include_str!("../wire/callbacks.rs");
        let lock = &callbacks[callbacks.find("ui.on_lock_screen(").expect("the lock callback")..];
        let lock = &lock[..lock.find("ui.set_current_screen(3)").expect("it shows the lock screen")];
        assert!(lock.contains("crate::capture::on_screen_lock()"), "the lock callback stops recordings first");
        // And logging out stops one before the session ends.
        let power = include_str!("../control_power.rs");
        let run = &power[power.find("pub fn run(verb: Verb)").expect("power's run")..];
        assert!(run[..run.find("verb.command()").unwrap()].contains("crate::capture::before_session_ends"), "logout stops it first");
    }

    /// Security review of #828, finding 4: "only yours" is by which mind, not by its name. Two
    /// agents with tokens were both "an agent", so either stopped the other's recording.
    #[test]
    fn a_mind_stops_only_the_recording_it_started() {
        let mind = |name: &str, id: Option<&str>| Who::Mind { name: name.into(), id: id.map(String::from) };
        let pi = mind("an agent", Some("agent:pi:c-1"));
        let other = mind("an agent", Some("agent:hermes:main"));
        assert!(pi.started_by(&pi.clone()));
        assert!(!pi.started_by(&other), "same name, another agent");
        assert!(!pi.started_by(&Who::Person), "the person stops it with Why::Person and no `only_by`");
        assert!(!mind("a mind", None).started_by(&mind("a mind", None)), "a mind the shell cannot tell owns nothing");
        assert!(Who::Person.started_by(&Who::Person));

        // And the stop itself refuses another agent, before any unit is touched.
        *state() = Some(Running {
            unit: "yantrik-capture-rec-test".into(),
            path: PathBuf::from("/nonexistent/Recording.mp4"),
            started: Instant::now(),
            wall: Local::now(),
            target: Target::MindView,
            audio: Audio::None,
            who: pi,
            encoder: Encoder::X264,
            stopping: false,
            retried: false,
        });
        let refused = stop(Why::Mind("an agent".into()), Some(&other));
        let still = state().as_ref().map(|r| r.stopping);
        *state() = None;
        assert!(refused.unwrap_err().contains("not started by you"));
        assert_eq!(still, Some(false), "left running, and not marked as stopping");
    }

    #[test]
    fn elapsed_time_reads_like_a_clock() {
        assert_eq!(elapsed_words(Duration::from_secs(0)), "0:00");
        assert_eq!(elapsed_words(Duration::from_secs(42)), "0:42");
        assert_eq!(elapsed_words(Duration::from_secs(723)), "12:03");
        assert_eq!(elapsed_words(Duration::from_secs(3723)), "1:02:03");
    }
}
