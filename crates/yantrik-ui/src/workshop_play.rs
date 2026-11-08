//! The OS's player for a file out of a workshop, in a sandbox of its own (#1124, M2).
//!
//! The file is hostile content: a decoder bug is code execution as whoever runs the decoder, and
//! a playlist or reference inside a "media" file can make a player fetch an address of its
//! choosing. So `mpv` is not started as the person with the network and the home. It is started
//! under `bwrap`:
//!
//! - no network and no home (`--unshare-all`, a tmpfs over `$HOME`), no capability, a new session;
//! - the system read-only, the display socket bound in, and nothing else. **No sound**: neither
//!   `pipewire-0` nor `pulse/native` is bound (#1124, round 5, finding 3). A native PipeWire client
//!   can record the microphone, and the pulse protocol can `load-module` into the person's
//!   `pipewire-pulse` (a network tunnel, a pipe sink that makes files), so either socket is a way out
//!   of the sandbox for a decoder bug. `workshop_present` plays without sound (`--ao=null`) until a
//!   safer audio path exists (a socket that refuses `load-module`, or the shell playing the audio);
//!   Stage needs that follow-up;
//! - the file as an inherited descriptor (`fd://3`), never a path;
//! - the demuxer pinned to the sniffed type (`workshop_out::demuxer`), playlists and references
//!   off, no configuration, no scripts, no `ytdl`, no resume file.
//!
//! With no `bwrap` nothing is shown: the player is never started unsandboxed.

use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

use crate::workshop_out::{checked_media, demuxer, mind_name};

/// `mpv`'s own words for a file of type `media`, named as coming from `line`'s workshop.
pub fn mpv_args(media: &str, line: &str) -> Vec<String> {
    [
        "--player-operation-mode=pseudo-gui",
        "--no-config",
        "--load-scripts=no",
        "--ytdl=no",
        "--load-unsafe-playlists=no",
        "--access-references=no",
        "--no-resume-playback",
        "--sub-auto=no",
        "--audio-file-auto=no",
        "--demuxer=lavf",
        // No audio output: no sound socket is in the sandbox, so none is tried.
        "--ao=null",
    ]
    .map(String::from)
    .into_iter()
    .chain([
        format!("--demuxer-lavf-format={}", demuxer(media)),
        format!("--force-media-title={line}"),
        format!("--osd-playing-msg={line}"),
        "--".into(),
        "fd://3".into(),
    ])
    .collect()
}

/// The `bwrap` words that wrap `mpv`, up to and including its name. `runtime` is
/// `$XDG_RUNTIME_DIR` and `home` the home folder; the display socket is bound only where it is.
pub fn sandbox_args(home: &Path, runtime: Option<&Path>, wayland: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = [
        "--die-with-parent", "--new-session", "--unshare-all", "--cap-drop", "ALL", "--clearenv",
        "--ro-bind", "/usr", "/usr", "--ro-bind-try", "/lib", "/lib", "--ro-bind-try", "/lib64", "/lib64",
        "--ro-bind-try", "/bin", "/bin", "--ro-bind-try", "/sbin", "/sbin", "--ro-bind", "/etc", "/etc",
        "--proc", "/proc", "--dev", "/dev", "--dev-bind-try", "/dev/dri", "/dev/dri", "--tmpfs", "/tmp",
        "--setenv", "PATH", "/usr/bin:/bin", "--setenv", "LC_ALL", "C.UTF-8",
    ]
    .map(String::from)
    .to_vec();
    let h = home.display().to_string();
    a.extend(["--tmpfs".into(), h.clone(), "--setenv".into(), "HOME".into(), h]);
    if let Some(run) = runtime {
        let r = run.display().to_string();
        a.extend(["--dir".into(), r.clone(), "--setenv".into(), "XDG_RUNTIME_DIR".into(), r]);
        let mut sockets: Vec<String> = Vec::new();
        if let Some(w) = wayland.filter(|w| !w.is_empty() && !w.contains('/') && *w != "..") {
            sockets.push(w.to_string());
            a.extend(["--setenv".into(), "WAYLAND_DISPLAY".into(), w.to_string()]);
        }
        for s in sockets {
            let p = run.join(&s).display().to_string();
            a.extend(["--ro-bind-try".into(), p.clone(), p]);
        }
    }
    a.push("--".into());
    a.push("mpv".into());
    a
}

/// Show the file in the OS's player, from its descriptor. The player's title says whose it is.
pub fn present(workshop: &Path, path: &Path, agent: &str) -> Result<String, String> {
    let (fd, media) = checked_media(workshop, path)?;
    let bwrap = yantrik_ipc_contracts::home_paths::workshops::find_bwrap()
        .ok_or("no media player can be started without bubblewrap (bwrap): the player is never run unsandboxed.")?;
    let line = format!("From {}'s workshop", mind_name(agent));
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/root".into()));
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute());
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let mut cmd = std::process::Command::new(bwrap);
    cmd.args(sandbox_args(&home, runtime.as_deref(), wayland.as_deref()))
        .args(mpv_args(media, &line))
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hand_over(&mut cmd, &fd);
    let mut child = cmd.spawn().map_err(|e| format!("no media player (mpv) could be started ({e})"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(media.to_string())
}

/// The file becomes descriptor 3 of the child, and of the player under `bwrap`, which keeps the
/// descriptors it inherits.
fn hand_over(cmd: &mut std::process::Command, fd: &OwnedFd) {
    let raw = fd.as_raw_fd();
    // Safety: only dup2 and fcntl between fork and exec.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(move || {
            if raw == 3 {
                let flags = libc::fcntl(3, libc::F_GETFD);
                if flags < 0 || libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(raw, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn has(a: &[String], w: &[&str]) -> bool {
        a.windows(w.len()).any(|x| x.iter().zip(w).all(|(a, b)| a == b))
    }

    #[test]
    fn the_player_has_no_network_no_home_and_no_capability() {
        let a = sandbox_args(Path::new("/home/p"), Some(Path::new("/run/user/1000")), Some("wayland-0"));
        assert!(a.iter().any(|x| x == "--unshare-all"), "network, pid, ipc, user and uts are all unshared");
        assert!(!a.iter().any(|x| x == "--share-net"));
        assert!(has(&a, &["--tmpfs", "/home/p"]) && has(&a, &["--cap-drop", "ALL"]) && a.iter().any(|x| x == "--clearenv"));
        assert!(a.iter().any(|x| x == "--new-session") && a.iter().any(|x| x == "--die-with-parent"));
        // The system is read-only; the only writable things are tmpfs.
        assert!(!a.iter().any(|x| x == "--bind" || x == "--dev-bind"), "{a:?}");
        // Only the sockets it plays through are bound in, never the runtime directory or the home.
        for w in a.windows(2).filter(|w| w[0] == "--ro-bind-try") {
            assert!(!["/run/user/1000", "/home/p", "/"].contains(&w[1].as_str()), "{}", w[1]);
        }
        assert!(has(&a, &["--ro-bind-try", "/run/user/1000/wayland-0", "/run/user/1000/wayland-0"]));
        // Finding 3: no sound socket, so the decoder cannot reach the microphone or `load-module`.
        assert!(!a.iter().any(|x| x.contains("pipewire") || x.contains("pulse")), "{a:?}");
        assert!(mpv_args("audio/mpeg", "x").contains(&"--ao=null".to_string()));
        assert_eq!(&a[a.len() - 2..], ["--", "mpv"]);
        // A socket name that climbs out of the runtime folder is not bound.
        let bad = sandbox_args(Path::new("/home/p"), Some(Path::new("/run/user/1000")), Some("../../../etc/passwd"));
        assert!(!bad.iter().any(|x| x.contains("passwd")));
    }

    #[test]
    fn the_demuxer_is_the_sniffed_one_and_references_and_playlists_are_off() {
        let m = mpv_args("audio/mpeg", "From Hermes's workshop");
        assert!(m.contains(&"--demuxer-lavf-format=mp3".to_string()) && m.contains(&"--demuxer=lavf".to_string()));
        for off in ["--no-config", "--load-scripts=no", "--ytdl=no", "--load-unsafe-playlists=no", "--access-references=no", "--no-resume-playback"] {
            assert!(m.contains(&off.to_string()), "{off}");
        }
        assert_eq!(&m[m.len() - 2..], ["--", "fd://3"], "the file is a descriptor, not a path");
        // An ID3 header is admitted as MP3 and read as MP3 only: it cannot become a playlist.
        let sniffed = crate::workshop_out::sniff(b"ID3\x04\0\0\0\0\0\x0a\0\0\0\0\0\0\0\0\0\0#EXTM3U\n").unwrap();
        assert_eq!(sniffed, "audio/mpeg");
        assert_eq!(demuxer(sniffed), "mp3");
        for t in ["video/mp4", "video/webm", "audio/ogg", "audio/wav", "image/webp", "video/avi", "audio/flac", "audio/mpeg", "image/png", "image/jpeg", "image/gif"] {
            assert!(!["hls", "applehttp", "concat", "sdp", "rtsp", "none"].contains(&demuxer(t)), "{t}");
        }
        assert_eq!(demuxer("text/plain"), "none");
    }
}
