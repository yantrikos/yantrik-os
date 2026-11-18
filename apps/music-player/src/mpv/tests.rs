//! The socket, the reports and the process, against stand-ins and, where it is installed, the
//! real mpv.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::client::{Client, Event};
use super::engine::{args, socket_path, Engine, StartError, REDIRECTED};
use super::observed::{Observed, Shared};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-music-mpv-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A stand-in for mpv: answers every command with `answer(command)`, first sending the events
/// `before(command)` returns, and records what it was sent.
fn fake_mpv(
    socket: &Path,
    answer: impl Fn(&Value) -> Value + Send + 'static,
    before: impl Fn(&Value) -> Vec<Value> + Send + 'static,
) -> Arc<Mutex<Vec<Value>>> {
    let listener = UnixListener::bind(socket).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            let v: Value = serde_json::from_str(&line).unwrap();
            log.lock().unwrap().push(v.clone());
            for event in before(&v["command"]) {
                writeln!(out, "{event}").unwrap();
            }
            let mut reply = answer(&v["command"]);
            reply["request_id"] = v["request_id"].clone();
            writeln!(out, "{reply}").unwrap();
            if v["command"][0] == "quit" {
                break;
            }
        }
    });
    seen
}

#[test]
fn a_command_is_sent_as_a_json_array_and_answered_by_its_request_id() {
    let dir = scratch("cmd");
    let socket = dir.join("mpv.sock");
    let seen = fake_mpv(
        &socket,
        |c| match c[0].as_str() {
            Some("get_property") if c[1] == "volume" => json!({"error": "success", "data": 64.0}),
            Some("get_property") => json!({"error": "property unavailable"}),
            _ => json!({"error": "success"}),
        },
        |_| vec![],
    );
    let client = Client::connect(&socket, |_| {}).unwrap();

    assert_eq!(client.get("volume"), Ok(json!(64.0)));
    assert_eq!(client.get("duration"), Err("mpv said: property unavailable".into()));

    // A name a shell would split, expand or run is one argument, untouched.
    let nasty = "/home/me/Music/$(rm -rf ~); `id` 'quoted' \"and\" spaced.mp3";
    client.load(nasty).unwrap();
    let sent = seen.lock().unwrap().clone();
    let load = sent.iter().find(|v| v["command"][0] == "loadfile").expect("loadfile was sent");
    assert_eq!(load["command"], json!(["loadfile", nasty, "replace"]));
    assert!(load["request_id"].as_u64().is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn events_are_sorted_from_replies_and_kept_as_what_mpv_reported() {
    let dir = scratch("events");
    let socket = dir.join("mpv.sock");
    fake_mpv(
        &socket,
        |_| json!({"error": "success"}),
        |c| {
            if c[0] == "loadfile" {
                vec![
                    json!({"event": "property-change", "id": 7, "name": "path", "data": c[1]}),
                    json!({"event": "file-loaded"}),
                    json!({"event": "property-change", "id": 2, "name": "duration", "data": 245.5}),
                    json!({"event": "property-change", "id": 1, "name": "time-pos", "data": 0.25}),
                    json!({"event": "property-change", "id": 6, "name": "idle-active", "data": false}),
                    json!({"event": "end-file", "reason": "eof"}),
                ]
            } else {
                vec![]
            }
        },
    );
    let shared = Arc::new(Shared::default());
    let (tx, rx) = mpsc::channel();
    let sink = shared.clone();
    let tx = Mutex::new(tx);
    let client = Client::connect(&socket, move |e| {
        sink.apply(&e);
        let _ = tx.lock().unwrap().send(e);
    })
    .unwrap();
    client.load("/music/a.flac").unwrap();
    let (ok, seen) = shared.wait_for(Duration::from_secs(2), |o| o.time_pos.is_some());
    assert!(ok, "{seen:?}");
    assert_eq!(seen.path.as_deref(), Some("/music/a.flac"));
    assert_eq!(seen.duration, Some(245.5));
    assert_eq!(seen.loads, 1);
    assert!(seen.playing());
    let events: Vec<Event> = rx.try_iter().collect();
    assert!(events.contains(&Event::EndFile { reason: "eof".into(), error: None }), "{events:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_player_that_goes_away_is_said_once_and_fails_waiting_commands() {
    let dir = scratch("gone");
    let socket = dir.join("mpv.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Read one command, then die without answering it.
        let mut buf = [0u8; 64];
        let _ = stream.read(&mut buf);
    });
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let client = Client::connect(&socket, move |e| {
        let _ = tx.lock().unwrap().send(e);
    })
    .unwrap();
    let reply = client.get("pause");
    assert_eq!(reply, Err("the player (mpv) stopped".into()));
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(Event::Gone));
    assert!(!client.is_alive());
    assert_eq!(client.get("pause"), Err("the player (mpv) is not running".into()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_player_is_said_in_plain_words() {
    let dir = scratch("missing");
    let err = Engine::start("/nonexistent/mpv-is-not-here", dir.join("s.sock"), 80.0, |_| {})
        .err()
        .expect("no such program");
    assert_eq!(err, StartError::Missing);
    assert!(err.to_string().contains("mpv, is not installed"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_error_at_the_end_of_a_file_is_kept_until_the_next_one_loads() {
    let mut o = Observed::default();
    o.apply(&Event::EndFile { reason: "error".into(), error: Some("unrecognized file format".into()) });
    assert_eq!(o.last_error.as_deref(), Some("unrecognized file format"));
    o.apply(&Event::FileLoaded);
    assert_eq!(o.last_error, None);
    o.apply(&Event::Gone);
    assert!(!o.playing());
}

#[test]
fn a_playlist_where_a_sound_file_should_be_counts_as_a_failure() {
    let mut o = Observed::default();
    o.apply(&Event::EndFile { reason: "redirect".into(), error: None });
    assert_eq!(o.errors, 1);
    assert_eq!(o.last_error.as_deref(), Some(REDIRECTED));
}

#[test]
fn mpv_is_told_never_to_follow_what_a_file_points_at() {
    let args = args(150.0, Path::new("/run/user/1000/m.sock"));
    for flag in [
        "--access-references=no",
        "--load-unsafe-playlists=no",
        "--ytdl=no",
        "--no-config",
        "--load-scripts=no",
        "--demuxer-lavf-o=protocol_whitelist=file,format_whitelist=[wav,mp3,flac,ogg,mov,aac]",
    ] {
        assert!(args.iter().any(|a| a == flag), "{flag} missing from {args:?}");
    }
    assert!(args.iter().any(|a| a.starts_with("--network-timeout=")), "{args:?}");
    assert!(args.contains(&"--volume=100".to_string()), "{args:?}");
    assert_eq!(args.last().unwrap(), "--input-ipc-server=/run/user/1000/m.sock");
}

/// The socket goes only where no other account can reach it, and each player gets its own name.
#[test]
fn the_socket_goes_only_in_a_private_runtime_directory_under_a_name_of_its_own() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("runtime");
    assert!(socket_path(None).unwrap_err().to_string().contains("XDG_RUNTIME_DIR is not set"));
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let open = socket_path(Some(&dir)).unwrap_err().to_string();
    assert!(open.contains("is not a directory only this account can open"), "{open}");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (a, b) = (socket_path(Some(&dir)).unwrap(), socket_path(Some(&dir)).unwrap());
    assert!(a.starts_with(&dir) && b.starts_with(&dir));
    assert_ne!(a, b, "a player starting never takes the name of one still going away");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The real thing, where the machine has it: the socket is 0600, a file plays, and mpv is gone
/// once the engine is dropped.
#[test]
fn a_real_mpv_runs_behind_a_private_socket_and_exits_with_the_engine() {
    if Command::new("mpv").arg("--version").stdout(Stdio::null()).status().is_err() {
        eprintln!("no mpv here; skipped");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("real");
    let socket = dir.join("mpv.sock");
    let engine = Engine::start("mpv", socket.clone(), 0.0, |_| {}).expect("mpv starts");
    let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the socket is this account's alone");
    assert_eq!(engine.client.get("volume"), Ok(json!(0.0)));
    let pid = engine.pid();
    drop(engine);
    assert!(!socket.exists(), "the socket goes with the engine");
    let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    assert!(state.is_empty() || state.contains(") Z"), "mpv is gone: {state}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A short, silent WAV that mpv plays.
fn wav(path: &Path) {
    let (rate, samples) = (8000u32, 4000u32);
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(samples * 2).to_le_bytes());
    b.resize(b.len() + samples as usize * 2, 0);
    std::fs::write(path, b).unwrap();
}

/// The security review's file: `evil.mp3` is an m3u playlist naming a URL and a file. A real
/// mpv refuses it without contacting the listener or opening the file, and a real sound file
/// after it still plays.
#[test]
fn a_real_mpv_does_not_follow_a_playlist_named_like_a_sound_file() {
    if Command::new("mpv").arg("--version").stdout(Stdio::null()).status().is_err() {
        eprintln!("no mpv here; skipped");
        return;
    }
    let dir = scratch("evil");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let outside = dir.join("outside.wav");
    wav(&outside);
    let evil = dir.join("evil.mp3");
    std::fs::write(&evil, format!("#EXTM3U\nhttp://127.0.0.1:{port}/beacon.mp3\n{}\n", outside.display())).unwrap();
    let good = dir.join("good.wav");
    wav(&good);

    let engine = Engine::start("mpv", dir.join("mpv.sock"), 0.0, |_| {}).expect("mpv starts");
    engine.client.load(evil.to_str().unwrap()).unwrap();
    let (ended, o) = engine.observed.wait_for(Duration::from_secs(5), |o| o.errors > 0 || o.loads > 0);
    assert!(ended, "mpv said something about evil.mp3: {o:?}");
    assert_eq!(o.loads, 0, "nothing played, neither the URL nor outside.wav: {o:?}");
    assert_ne!(o.last_error.as_deref(), Some(REDIRECTED), "mpv did not even open it as a playlist");
    // Give a fetch that slipped through time to arrive.
    std::thread::sleep(Duration::from_millis(500));
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("mpv contacted the address in the playlist: {other:?}"),
    }

    engine.client.load(good.to_str().unwrap()).unwrap();
    let want = good.to_str().unwrap().to_string();
    let (played, o) =
        engine.observed.wait_for(Duration::from_secs(5), |o| o.loads > 0 && o.path.as_deref() == Some(want.as_str()));
    assert!(played, "a real sound file still plays: {o:?}");
    drop(engine);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The security delta review's file: `sdp.mp3` is an SDP description, which ffmpeg would open
/// as a live RTP stream on UDP ports of every interface. A real mpv refuses it, binds neither
/// port, and a real sound file after it still plays.
#[test]
fn a_real_mpv_does_not_open_network_ports_for_an_sdp_file_named_like_a_sound_file() {
    if Command::new("mpv").arg("--version").stdout(Stdio::null()).status().is_err() {
        eprintln!("no mpv here; skipped");
        return;
    }
    use std::net::UdpSocket;
    let dir = scratch("sdp");
    // An even port whose next one is free too: RTP takes the port, RTCP the one after it.
    let port = (0..50)
        .find_map(|_| {
            let p = UdpSocket::bind("0.0.0.0:0").ok()?.local_addr().ok()?.port() & !1;
            (p > 1024 && UdpSocket::bind(("0.0.0.0", p)).is_ok() && UdpSocket::bind(("0.0.0.0", p + 1)).is_ok())
                .then_some(p)
        })
        .expect("a free pair of UDP ports");
    let sdp = dir.join("sdp.mp3");
    std::fs::write(&sdp, format!("v=0\no=- 0 0 IN IP4 127.0.0.1\ns=x\nc=IN IP4 127.0.0.1\nt=0 0\nm=audio {port} RTP/AVP 0\n"))
        .unwrap();
    let good = dir.join("good.wav");
    wav(&good);

    let engine = Engine::start("mpv", dir.join("mpv.sock"), 0.0, |_| {}).expect("mpv starts");
    engine.client.load(sdp.to_str().unwrap()).unwrap();
    let (ended, o) = engine.observed.wait_for(Duration::from_secs(5), |o| o.errors > 0 || o.loads > 0);
    // Checked while this mpv still runs: a socket it bound would still be held.
    for p in [port, port + 1] {
        assert!(UdpSocket::bind(("0.0.0.0", p)).is_ok(), "mpv bound UDP port {p} for sdp.mp3: {o:?}");
    }
    assert!(ended, "mpv refused sdp.mp3 instead of waiting on the network: {o:?}");
    assert_eq!(o.loads, 0, "nothing played: {o:?}");

    engine.client.load(good.to_str().unwrap()).unwrap();
    let want = good.to_str().unwrap().to_string();
    let (played, o) =
        engine.observed.wait_for(Duration::from_secs(5), |o| o.loads > 0 && o.path.as_deref() == Some(want.as_str()));
    assert!(played, "a real sound file still plays: {o:?}");
    drop(engine);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file that loaded is found by a waiter however soon the next one followed: mpv may load a
/// file and be handed the next before the waiter looks (#937 delta review, finding A).
#[test]
fn a_file_that_loaded_is_found_though_the_next_one_followed_before_anyone_looked() {
    let mut o = Observed::default();
    for event in [
        Event::Property("path".into(), json!("/m/slow.mp3")),
        Event::Property("duration".into(), json!(200.0)),
        Event::FileLoaded,
        Event::EndFile { reason: "stop".into(), error: None },
        Event::Property("path".into(), json!("/m/next.mp3")),
        Event::FileLoaded,
    ] {
        o.apply(&event);
    }
    assert_eq!(o.path.as_deref(), Some("/m/next.mp3"), "the latest state has moved on");
    let at = o.loaded_since(0, "/m/slow.mp3").expect("the history has it");
    assert_eq!((at.path.as_deref(), at.duration), (Some("/m/slow.mp3"), Some(200.0)));
    assert!(o.loaded_since(1, "/m/slow.mp3").is_none(), "not a load from before the wait");
    for n in 0..20 {
        o.apply(&Event::Property("path".into(), json!(format!("/m/{n}.mp3"))));
        o.apply(&Event::FileLoaded);
    }
    assert_eq!(o.loaded.len(), 8, "only the last few are kept");
}
