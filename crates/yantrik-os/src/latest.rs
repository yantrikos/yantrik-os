//! A worker that does only the newest of what it was asked.
//!
//! A slider drag asks for a new level dozens of times a second, and each level is a process
//! to run (`wpctl`, `brightnessctl`). Running them all on the thread that draws the screen
//! stalls it; running each on a thread of its own lets them finish out of order and leave the
//! machine at an old level. One worker, which skips to the latest value waiting, does neither.

use crossbeam_channel::Sender;

/// Start a worker thread that runs `apply` on values sent to the returned sender, skipping any
/// value that a newer one has already replaced.
pub fn spawn_latest<T, F>(name: &str, apply: F) -> Sender<T>
where
    T: Send + 'static,
    F: Fn(T) + Send + 'static,
{
    let (tx, rx) = crossbeam_channel::unbounded::<T>();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        while let Ok(mut value) = rx.recv() {
            while let Ok(newer) = rx.try_recv() {
                value = newer;
            }
            apply(value);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, worker = name, "could not start the worker; changes will not be applied");
    }
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn a_burst_ends_on_its_last_value() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let (s, g) = (seen.clone(), gate.clone());
        let tx = spawn_latest("test-latest", move |v: u32| {
            let _wait = g.lock().unwrap();
            s.lock().unwrap().push(v);
        });
        // The first value is picked up and held at the gate; the rest queue behind it.
        tx.send(1).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        for v in 2..=9 {
            tx.send(v).unwrap();
        }
        drop(held);
        drop(tx);
        std::thread::sleep(Duration::from_millis(200));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&1));
        assert_eq!(seen.last(), Some(&9), "the machine ends at the last level asked for");
        assert!(seen.len() <= 3, "the levels in between were skipped: {seen:?}");
    }
}
