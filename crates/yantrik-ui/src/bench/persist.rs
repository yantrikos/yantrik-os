//! Writing the layout file and the Continue record without making the pointer wait for the disk.
//!
//! A drop writes `~/.yantrik/desktop_layout.json`, and that write ends in an fsync, which on a
//! slow disk is long enough to see as a stutter at the end of every drag. So the shell hands a
//! copy to one writer thread that sleeps until it has something, writes the newest layout and
//! skips what was overtaken. One thread writes both files, so two writes never meet. Tests write
//! in the calling thread instead, so what they read back is what was just written.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use yantrik_bench::layout::Store;
use yantrik_bench::recent::{self, LastOpened};

pub enum Msg {
    Layout(Store),
    Recent(LastOpened),
}

pub enum Saver {
    /// In the caller's thread. For tests.
    Direct { layout: PathBuf, recent: PathBuf },
    /// On the writer thread.
    Worker(mpsc::Sender<Msg>),
}

impl Saver {
    /// A writer thread for these two files. It lives as long as the sender does.
    pub fn worker(layout: PathBuf, record: PathBuf) -> Saver {
        let (tx, rx) = mpsc::channel::<Msg>();
        let spawned = std::thread::Builder::new().name("bench-writer".into()).spawn(move || {
            // Blocks until there is something to write; no timer, nothing polled.
            while let Ok(first) = rx.recv() {
                let (mut layout_now, mut recent_now) = (None, None);
                let mut take = |m: Msg| match m {
                    Msg::Layout(s) => layout_now = Some(s),
                    Msg::Recent(r) => recent_now = Some(r),
                };
                take(first);
                while let Ok(next) = rx.try_recv() {
                    take(next);
                }
                if let Some(mut s) = layout_now {
                    if let Err(e) = s.save(&layout) {
                        tracing::warn!(error = %e, "The desktop layout was not saved");
                    }
                }
                if let Some(r) = recent_now {
                    if let Err(e) = recent::save(&record, &r) {
                        tracing::warn!(error = %e, "The last-opened record was not saved");
                    }
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "The desktop's writer could not start; the layout will not be kept");
        }
        Saver::Worker(tx)
    }

    pub fn save(&self, store: &mut Store) {
        match self {
            Saver::Direct { layout, .. } => {
                if let Err(e) = store.save(layout) {
                    tracing::warn!(error = %e, "The desktop layout was not saved");
                }
            }
            Saver::Worker(tx) => {
                if tx.send(Msg::Layout(store.clone())).is_ok() {
                    store.handed_over();
                }
            }
        }
    }

    pub fn keep_recent(&self, record: &LastOpened) {
        match self {
            Saver::Direct { recent: path, .. } => {
                let _ = recent::save(path, record);
            }
            Saver::Worker(tx) => {
                let _ = tx.send(Msg::Recent(record.clone()));
            }
        }
    }
}

/// Where the layout lives: `~/.yantrik/desktop_layout.json`.
pub fn layout_path(home: &Path) -> PathBuf {
    home.join(".yantrik").join("desktop_layout.json")
}

/// Where the Continue row's record lives: `~/.yantrik/last_opened.json`.
pub fn recent_path(home: &Path) -> PathBuf {
    home.join(".yantrik").join("last_opened.json")
}
