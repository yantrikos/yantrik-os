//! A process that has nothing to do with a PTY forks while shells are being started, again and
//! again: none of its children may come up holding a PTY master. The master is close-on-exec from
//! the moment it exists, so no fork anywhere in the process can copy it.
//!
//! This is a test binary of its own on purpose. It keeps three threads forking and starts 300
//! shells, which takes every core for a few seconds; cargo runs test binaries one after another,
//! so it cannot starve the flood test of the library's suite.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use yantrik_terminal::Session;

#[test]
fn no_forking_thread_ever_inherits_a_pty_master() {
    let stop = Arc::new(AtomicBool::new(false));
    let children = Arc::new(AtomicUsize::new(0));
    let leaks = Arc::new(Mutex::new(Vec::<String>::new()));
    let forkers: Vec<_> = (0..3)
        .map(|_| {
            let (stop, children, leaks) = (stop.clone(), children.clone(), leaks.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let out = std::process::Command::new("/bin/sh")
                        .args(["-c", "for f in /proc/$$/fd/*; do readlink \"$f\"; done"])
                        .output()
                        .unwrap();
                    children.fetch_add(1, Ordering::Relaxed);
                    let seen = String::from_utf8_lossy(&out.stdout).into_owned();
                    if seen.contains("ptmx") {
                        leaks.lock().unwrap().push(seen);
                    }
                }
            })
        })
        .collect();
    for _ in 0..300 {
        Session::spawn_shell("/bin/sh", &["-i"], Path::new("/tmp"), 24, 80, Arc::new(|| {})).unwrap().shutdown();
    }
    stop.store(true, Ordering::Relaxed);
    forkers.into_iter().for_each(|t| t.join().unwrap());
    let leaks = leaks.lock().unwrap();
    assert!(leaks.is_empty(), "{} of {} children held a ptmx", leaks.len(), children.load(Ordering::Relaxed));
}
