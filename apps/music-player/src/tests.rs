//! The real window and its control surface, against a stand-in mpv.
//!
//! The window is a `MinimalSoftwareWindow` behind a substituted platform, so this runs headless;
//! the stand-in is a small Python program that speaks mpv's JSON IPC on the socket the app hands
//! it, so the app's whole path — spawn, socket, observe, loadfile, events, auto-advance, death —
//! is the one that runs against the real player. `mpv.rs` tests the real mpv where it is
//! installed.
//!
//! One `#[test]` for the window, because a process may set Slint's platform only once.

use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use std::collections::VecDeque;
use std::time::Instant;

type Jobs = Arc<Mutex<VecDeque<Box<dyn FnOnce() + Send>>>>;

struct Proxy(Jobs);
impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), slint::EventLoopError> {
        self.0.lock().unwrap().push_back(event);
        Ok(())
    }
}

struct Headless {
    window: Rc<MinimalSoftwareWindow>,
    jobs: Jobs,
}
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.jobs.clone())))
    }
}

/// Run what other threads handed the event loop, until `done` holds or six seconds pass.
fn pump(jobs: &Jobs, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let batch: Vec<_> = jobs.lock().unwrap().drain(..).collect();
        for job in batch {
            job();
        }
        if done() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Speaks enough of mpv's JSON IPC for the app: observe, get/set, loadfile, seek, stop, quit.
/// A file whose name holds `short` ends as soon as it starts; `broken` fails to open; `crash`
/// takes the player down; `playlist` behaves like an mpv that follows a playlist: it reports a
/// `redirect` and starts playing the URL inside; `elsewhere` loads another file than the one asked
/// for, one outside the home.
const FAKE_MPV: &str = r#"
import json, os, socket, sys
path = [a.split('=', 1)[1] for a in sys.argv if a.startswith('--input-ipc-server=')][0]
vol = float([a.split('=', 1)[1] for a in sys.argv if a.startswith('--volume=')][0])
srv = socket.socket(socket.AF_UNIX)
srv.bind(path)
srv.listen(1)
conn, _ = srv.accept()
f = conn.makefile('rwb', buffering=0)
props = {'time-pos': None, 'duration': None, 'pause': False, 'volume': vol, 'mute': False, 'idle-active': True, 'path': None}
observed = {}
def send(o):
    f.write((json.dumps(o) + '\n').encode())
def change(name, v):
    props[name] = v
    if name in observed:
        send({'event': 'property-change', 'id': observed[name], 'name': name, 'data': v})
for line in f:
    m = json.loads(line)
    cmd, rid, reply, after = m['command'], m.get('request_id'), {'error': 'success'}, []
    n = cmd[0]
    if n == 'observe_property':
        observed[cmd[2]] = cmd[1]
        after.append({'event': 'property-change', 'id': cmd[1], 'name': cmd[2], 'data': props[cmd[2]]})
    elif n == 'get_property':
        if props.get(cmd[1]) is None:
            reply = {'error': 'property unavailable'}
        else:
            reply['data'] = props[cmd[1]]
    elif n == 'set_property':
        change(cmd[1], cmd[2])
    elif n == 'loadfile':
        p = cmd[1]
        if 'crash' in p:
            os._exit(3)
        if props['path']:
            send({'event': 'end-file', 'reason': 'stop'})
        if 'playlist' in p:
            after.append({'event': 'end-file', 'reason': 'redirect'})
            change('path', 'http://127.0.0.1:9/beacon.mp3'); change('idle-active', False)
            after.append({'event': 'file-loaded'})
        elif 'elsewhere' in p:
            change('path', '/var/elsewhere/secret.mp3'); change('idle-active', False)
            after.append({'event': 'file-loaded'})
        elif 'broken' in p:
            change('path', None); change('idle-active', True)
            after.append({'event': 'end-file', 'reason': 'error', 'file_error': 'unrecognized file format'})
        else:
            change('path', p); change('idle-active', False)
            after.append({'event': 'file-loaded'})
            change('duration', 200.0); change('time-pos', 0.0)
            if 'short' in p:
                after.append({'event': 'end-file', 'reason': 'eof'})
    elif n == 'seek':
        change('time-pos', float(cmd[1]))
    elif n == 'stop':
        change('path', None); change('idle-active', True)
        send({'event': 'end-file', 'reason': 'stop'})
    reply['request_id'] = rid
    send(reply)
    for event in after:
        send(event)
    if n == 'quit':
        break
"#;

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-music-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn actions(s: &State) -> HashMap<String, (Action, Handler)> {
    surface(s).into_iter().map(|(a, h)| (a.name.clone(), (a, h))).collect()
}
use std::collections::HashMap;

#[test]
fn the_window_plays_scans_and_answers_with_what_mpv_reported() {
    if std::process::Command::new("python3").arg("--version").output().is_err() {
        eprintln!("no python3 for the stand-in player; skipped");
        return;
    }
    let jobs: Jobs = Arc::new(Mutex::new(VecDeque::new()));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(slint::PhysicalSize::new(1100, 720));
    slint::platform::set_platform(Box::new(Headless { window: window.clone(), jobs: jobs.clone() })).unwrap();

    let dir = scratch();
    // The socket goes in the runtime directory, which must be private.
    let runtime = dir.join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::env::set_var("XDG_RUNTIME_DIR", &runtime);
    let fake = dir.join("fake-mpv");
    std::fs::write(&fake, format!("#!/usr/bin/env python3\n{FAKE_MPV}")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // A library of 300 tracks in three albums, a symlink loop, and a file that is not audio.
    let music = dir.join("Music");
    for album in ["Alpha", "Beta", "Gamma"] {
        std::fs::create_dir_all(music.join(album)).unwrap();
        for n in 0..100 {
            std::fs::write(music.join(album).join(format!("{album} {n:03}.mp3")), b"").unwrap();
        }
    }
    std::os::unix::fs::symlink(&music, music.join("Alpha/loop")).unwrap();
    std::fs::write(music.join("cover.jpg"), b"").unwrap();
    let store = dir.join("music.json");
    std::fs::write(&store, serde_json::to_vec(&json!({"folders": [music], "volume": 40.0})).unwrap()).unwrap();

    let app = MusicPlayerApp::new().unwrap();
    let s = wire(&app, Music::new(app.as_weak(), store.clone(), "/nonexistent/no-mpv-here"));
    let acts = actions(&s);
    // As the socket dispatches: the handler runs on the UI thread, and whatever it left to
    // `answer_later` runs on another while the UI thread keeps turning.
    let call = |name: &str, args: Value| -> Result<Value, String> {
        let scope = yantrik_surface::LaterScope::enter();
        let first = (acts[name].1)(&args);
        let Some(later) = scope.take() else { return first };
        drop(scope);
        let worker = std::thread::spawn(later);
        assert!(pump(&jobs, || worker.is_finished()), "`{name}` answered in time");
        worker.join().unwrap()
    };
    // The same, for an agent: its token is in scope only while the handler runs on the UI
    // thread, as in `ControlRpc::dispatch`, and not where the rest of the answer is built.
    let agent_call = |name: &str, args: Value| -> Result<Value, String> {
        let scope = yantrik_surface::LaterScope::enter();
        let first = {
            let _agent = yantrik_surface::AgentTokenScope::enter(Some("tok-test-agent".into()));
            (acts[name].1)(&args)
        };
        let Some(later) = scope.take() else { return first };
        drop(scope);
        let worker = std::thread::spawn(later);
        assert!(pump(&jobs, || worker.is_finished()), "`{name}` answered in time");
        worker.join().unwrap()
    };
    let dir_text = dir.to_string_lossy().into_owned();

    // ── Every published action is graded and says what it does ──
    for name in ["play", "pause", "next", "previous", "seek", "set_volume", "play_file", "rescan"] {
        let (spec, _) = acts.get(name).unwrap_or_else(|| panic!("`{name}` is published"));
        assert_eq!(spec.permission, "standard", "{name}");
        assert!(spec.description.len() > 40, "{name}: {}", spec.description);
    }

    // ── The scan runs off the UI thread, and its last sentence stays on screen ──
    start_scan(&s, None).unwrap();
    assert!(app.get_is_scanning(), "the window says it is scanning while the worker walks");
    assert!(app.get_status_text().contains("Scanning"), "{}", app.get_status_text());
    assert!(pump(&jobs, || !s.borrow().scanning), "the scan finished");
    assert_eq!(app.get_status_text(), "300 tracks in 1 folder");
    assert_eq!(app.get_library_track_count(), 300, "each song once, the loop not walked");
    let folders = app.get_scan_folders();
    assert_eq!(slint::Model::row_data(&folders, 0).unwrap().track_count, 300, "the folder's count is real");

    // A rescan through the surface answers with what was found.
    let found = call("rescan", json!({})).unwrap();
    assert_eq!(found["library_size"], 300);
    assert_eq!(found["folders"][0]["tracks"], 300);

    // ── No mpv: said in plain words, on screen and to the caller ──
    let err = call("play", json!({})).unwrap_err();
    assert!(err.contains("mpv, is not installed"), "{err}");
    assert_eq!(app.get_notice().to_string(), err);
    assert_eq!(view(&s).state["player"], "unavailable");

    // ── With a player: play, pause, seek, volume, next, previous — as mpv reports them ──
    s.borrow_mut().program = fake.to_string_lossy().into_owned();
    let played = call("play", json!({"index": 2})).unwrap();
    assert_eq!(played["reported_by"], "mpv");
    assert_eq!(played["playing"], true, "{played}");
    assert_eq!(played["track"], "Alpha 001");
    assert_eq!(played["volume"], 40.0, "mpv started at the kept volume");
    assert!(app.get_notice().is_empty(), "the old complaint about mpv is gone: {}", app.get_notice());
    pump(&jobs, || app.get_is_playing());
    assert_eq!(app.get_now_title(), "Alpha 001");
    assert_eq!(app.get_time_total(), "3:20");
    assert_eq!(app.get_queue_current_index(), 1);
    let socket = runtime.join(format!("yantrik-music-mpv.{}.sock", std::process::id()));
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the player's socket is this account's alone");
    }

    let paused = call("pause", json!({})).unwrap();
    assert_eq!(paused["paused"], true);
    pump(&jobs, || !app.get_is_playing());
    let resumed = call("play", json!({})).unwrap();
    assert_eq!(resumed["playing"], true);
    assert_eq!(resumed["path"].as_str().map(|p| p.ends_with("Alpha 001.mp3")), Some(true), "it resumed, not restarted");

    let sought = call("seek", json!({"seconds": 50.0})).unwrap();
    assert_eq!(sought["position"], 50.0);
    pump(&jobs, || app.get_time_current() == "0:50");
    assert!((app.get_progress() - 0.25).abs() < 0.01, "the bar is time-pos over duration");
    assert!(call("seek", json!({"seconds": 999.0})).unwrap_err().contains("past its end"));

    let loud = call("set_volume", json!({"percent": 65})).unwrap();
    assert_eq!(loud["volume"], 65.0);
    pump(&jobs, || (app.get_volume() - 0.65).abs() < 0.001);
    assert!(call("set_volume", json!({"percent": 140})).is_err());
    let kept: Value = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
    assert_eq!(kept["volume"], 65.0, "the volume is kept for next time");

    let next = call("next", json!({})).unwrap();
    assert_eq!(next["track"], "Alpha 002");
    // Under three seconds in, previous goes to the track before.
    let back = call("previous", json!({})).unwrap();
    assert_eq!(back["track"], "Alpha 001");

    let d = view(&s).state;
    assert_eq!(d["track"], "Alpha 001");
    assert_eq!(d["playing"], true);
    assert_eq!(d["duration"], 200.0);
    assert_eq!(d["volume"], 65.0);
    assert_eq!(d["queue_length"], 300);
    assert_eq!(d["queue_position"], 2);
    assert_eq!(d["library_size"], 300);
    assert_eq!(d["player"], "running");

    // ── The search filter narrows the list the window shows, and `index` follows it ──
    app.invoke_browse_filter_changed("gamma 01".into());
    assert_eq!(slint::Model::row_count(&app.get_library_tracks()), 10);
    app.invoke_browse_filter_changed("".into());

    // ── Auto-advance: a track that ends plays the next one ──
    let short = dir.join("Music/Beta/short one.mp3");
    std::fs::write(&short, b"").unwrap();
    let next_after = dir.join("Music/Beta/after.mp3");
    std::fs::write(&next_after, b"").unwrap();
    {
        let mut m = s.borrow_mut();
        let tracks = vec![library::track_from(&short), library::track_from(&next_after)];
        m.queue.start(tracks, 0);
    }
    let t = s.borrow().queue.current().cloned().unwrap();
    load(&s, &t).unwrap();
    assert!(
        pump(&jobs, || s.borrow().observed().path.as_deref() == Some(next_after.to_str().unwrap())),
        "the end of `short one` played `after`"
    );
    assert_eq!(s.borrow().queue.current_index(), Some(1));

    // ── play_file: a file by path is the whole queue; what is not a sound file is refused ──
    let one = call("play_file", json!({"path": dir.join("Music/Gamma/Gamma 007.mp3")})).unwrap();
    assert_eq!(one["track"], "Gamma 007");
    assert_eq!(view(&s).state["queue_length"], 1);
    let refused = call("play_file", json!({"path": music.join("cover.jpg")})).unwrap_err();
    assert!(refused.contains("not a sound file"), "{refused}");
    assert_eq!(app.get_notice().to_string(), refused, "and the window says so too");
    assert!(call("play_file", json!({"path": dir.join("nothing.mp3")})).unwrap_err().contains("no file"));
    let broken = dir.join("Music/broken.mp3");
    std::fs::write(&broken, b"").unwrap();
    let err = call("play_file", json!({"path": broken})).unwrap_err();
    assert!(err.contains("unrecognized file format"), "what mpv said is what the caller hears: {err}");
    // An old failure is not the answer to the next load.
    let good = call("play_file", json!({"path": dir.join("Music/Gamma/Gamma 008.mp3")})).unwrap();
    assert_eq!(good["track"], "Gamma 008", "{good}");

    // ── A ".mp3" that is a playlist: refused, stopped, and never reported as playing ──
    let playlist = dir.join("Music/playlist.mp3");
    std::fs::write(&playlist, b"#EXTM3U\nhttp://127.0.0.1:9/beacon.mp3\n").unwrap();
    let err = call("play_file", json!({"path": playlist})).unwrap_err();
    assert!(err.contains("playlist"), "the answer says what mpv did: {err}");
    assert!(
        pump(&jobs, || s.borrow().observed().path.is_none()),
        "what the playlist pointed at was stopped: {:?}",
        s.borrow().observed()
    );
    assert!(app.get_notice().to_string().contains("playlist"), "{}", app.get_notice());
    assert!(!app.get_is_playing());
    let good = call("play_file", json!({"path": dir.join("Music/Gamma/Gamma 009.mp3")})).unwrap();
    assert_eq!(good["track"], "Gamma 009", "{good}");

    // ── An agent is never told a path it may not read, in any answer ──
    // The library is under the temporary directory, outside the home: the person sees the
    // paths, an agent does not, though its answer is finished off the UI thread.
    let mine = call("play", json!({"index": 3})).unwrap();
    assert!(mine["path"].as_str().is_some_and(|p| p.ends_with("Alpha 002.mp3")), "{mine}");
    let theirs = agent_call("play", json!({"index": 3})).unwrap();
    assert_eq!(theirs["track"], "Alpha 002", "{theirs}");
    assert_eq!(theirs["path"], "(hidden)", "{theirs}");
    assert!(!theirs.to_string().contains(&dir_text), "{theirs}");
    for (name, args) in [("pause", json!({})), ("play", json!({})), ("seek", json!({"seconds": 1.0}))] {
        let answer = agent_call(name, args).unwrap();
        assert!(!answer.to_string().contains(&dir_text), "{name}: {answer}");
    }
    let scanned = agent_call("rescan", json!({})).unwrap();
    assert_eq!(scanned["folders"][0]["path"], "(hidden)", "{scanned}");
    assert!(!scanned.to_string().contains(&dir_text), "{scanned}");
    assert!(pump(&jobs, || !s.borrow().scanning));
    {
        let _agent = yantrik_surface::AgentTokenScope::enter(Some("tok-test-agent".into()));
        let d = view(&s).state;
        assert!(!d.to_string().contains(&dir_text), "describe hides them too: {d}");
    }
    // mpv loading another file than the one asked for: the person hears which, an agent does not.
    let elsewhere = dir.join("elsewhere.mp3");
    std::fs::write(&elsewhere, b"").unwrap();
    let err = call("play_file", json!({"path": elsewhere})).unwrap_err();
    assert!(err.contains("mpv is playing /var/elsewhere/secret.mp3 instead"), "{err}");
    // An agent may name only a file in the home, so this part needs one there.
    let in_home = home().join(format!("yantrik-music-test-{}", std::process::id()));
    if std::fs::create_dir_all(&in_home).is_ok() {
        let asked = in_home.join("elsewhere.mp3");
        std::fs::write(&asked, b"").unwrap();
        let err = agent_call("play_file", json!({"path": asked})).unwrap_err();
        let _ = std::fs::remove_dir_all(&in_home);
        assert!(err.contains("mpv is playing (hidden) instead"), "{err}");
        assert!(!err.contains("/var/elsewhere"), "{err}");
    } else {
        eprintln!("the home is not writable here; the agent's play_file case is skipped");
    }

    // ── The player dies: said in plain words, and the next play starts it again ──
    let crash = dir.join("Music/crash.mp3");
    std::fs::write(&crash, b"").unwrap();
    let _ = call("play_file", json!({"path": crash}));
    assert!(pump(&jobs, || s.borrow().player == Player::Stopped), "the death was noticed");
    assert_eq!(app.get_notice(), "The player (mpv) stopped. Press play to start it again.");
    assert!(!app.get_is_playing());
    assert_eq!(view(&s).state["player"], "stopped");
    let again = call("play_file", json!({"path": dir.join("Music/Beta/Beta 003.mp3")})).unwrap();
    assert_eq!(again["playing"], true, "a new mpv: {again}");
    assert_eq!(view(&s).state["player"], "running");

    // ── Folders: the dialog's words, plurals and refusals ──
    let music_text = std::fs::canonicalize(&music).unwrap().to_string_lossy().into_owned();
    assert!(add_folder(&s, &music_text).unwrap_err().ends_with("is already in the list."));
    assert!(add_folder(&s, "relative/path").unwrap_err().contains("full path"));
    assert!(add_folder(&s, dir.join("cover.jpg").to_str().unwrap()).unwrap_err().starts_with("Not a folder"));
    let solo = dir.join("Solo");
    std::fs::create_dir_all(&solo).unwrap();
    std::fs::write(solo.join("only.flac"), b"").unwrap();
    add_folder(&s, solo.to_str().unwrap()).unwrap();
    assert!(pump(&jobs, || !s.borrow().scanning));
    // 300, the five files the player tests added under Music, and Solo/only.flac. The scan goes by
    // name, so playlist.mp3 is listed; it is mpv that refuses it.
    assert_eq!(app.get_status_text(), "306 tracks in 2 folders");
    let kept: Value = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
    assert_eq!(kept["folders"].as_array().map(Vec::len), Some(2), "the folder list is kept");
    remove_folder(&s, 0).unwrap();
    assert!(pump(&jobs, || !s.borrow().scanning));
    assert_eq!(app.get_status_text(), "1 track in 1 folder");

    // ── mpv exits with the app ──
    let pid_alive = std::path::Path::new(&socket).exists();
    assert!(pid_alive, "the socket is there while the player runs");
    let engine = s.borrow_mut().engine.take();
    drop(engine);
    assert!(!socket.exists(), "the player and its socket went with the app");

    drop(acts);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every control the window draws is backed: each callback the screen declares is wired to a
/// handler in `wire`, and nothing the screen used to draw over nothing is back.
#[test]
fn no_control_on_the_screen_is_a_drawing() {
    let screen: String = include_str!("../../../crates/yantrik-ui-slint/ui/music_player.slint")
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = include_str!("main.rs");
    for line in screen.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("callback ") {
            let name = rest.split(['(', ';']).next().unwrap().trim().replace('-', "_");
            assert!(source.contains(&format!("on_{name}(")), "`{name}` is drawn and not wired");
        }
    }
    for gone in ["playlist", "shuffle", "repeat", "equalizer", "playback-speed", "folder-watch", "ai-explain"] {
        assert!(!screen.contains(gone), "`{gone}` is back on the screen with nothing behind it");
    }
}
