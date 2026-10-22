//! A stand-in for mpv: a small Python program that speaks enough of mpv's JSON IPC for the app,
//! on the socket the app hands it. The app's whole path — spawn, socket, observe, loadfile,
//! events, auto-advance, death, quit — is the one that runs against the real player; `mpv/`
//! tests the real mpv where it is installed.
//!
//! A file whose name holds `short` ends as soon as it starts; `broken` fails to open; `crash`
//! takes the player down; `playlist` behaves like an mpv that follows a playlist (it reports a
//! `redirect` and starts playing the URL inside); `elsewhere` loads another file than the one
//! asked for, one outside the home; `slow` is said to have loaded only when the next file is
//! asked for, as a player still opening one file when it is handed another; `stuck` is taken
//! and never said to have loaded.

use std::path::{Path, PathBuf};

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
deferred = []
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
        for event in deferred:
            send(event)
        deferred = []
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
            if 'stuck' not in p:
                (deferred if 'slow' in p else after).append({'event': 'file-loaded'})
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

/// Whether this machine can run the stand-in.
pub fn python() -> bool {
    std::process::Command::new("python3").arg("--version").output().is_ok()
}

/// The stand-in, written into `dir` as an executable.
pub fn install(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let fake = dir.join("fake-mpv");
    std::fs::write(&fake, format!("#!/usr/bin/env python3\n{FAKE_MPV}")).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    fake
}

/// Whether process `pid` has gone and been reaped: no zombie is left either.
pub fn reaped(pid: u32) -> bool {
    !Path::new(&format!("/proc/{pid}")).exists()
}
