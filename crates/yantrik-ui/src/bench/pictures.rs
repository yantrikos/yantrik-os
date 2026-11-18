//! The pictures on the icons, made off the UI thread.
//!
//! One worker thread blocks on a channel (no timer, no polling) and, for each file it is asked
//! about, calls `yantrik_bench::thumbs::load_or_make`, which reads the cache or cuts the picture
//! once and files it. The pixels come back through the event loop and are put on the icon; until
//! then, and for anything that cannot be cut, the icon shows its glyph. Nothing here is called
//! while the pointer is down or the desktop is being drawn.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};
use yantrik_bench::items::Item;
use yantrik_bench::thumbs;

use crate::App;

/// The cache stays under this many bytes; the oldest thumbnails go first.
const CACHE_LIMIT: u64 = 96 << 20;

struct Request {
    name: String,
    path: PathBuf,
    key: String,
}

/// What is known about the pictures: the ones made, the ones asked for and the ones that cannot
/// be cut. All keyed by the thumbnail's own key, so an edited file is a new question.
pub struct Pictures {
    tx: Option<mpsc::Sender<Request>>,
    ready: HashMap<String, (String, Image)>,
    asked: HashSet<String>,
    failed: HashSet<String>,
}

impl Pictures {
    /// Start the worker. `cache` is `~/.cache/yantrik/thumbs`.
    pub fn start(cache: PathBuf, ui: slint::Weak<App>) -> Pictures {
        let (tx, rx) = mpsc::channel::<Request>();
        let spawned = std::thread::Builder::new().name("bench-thumbs".into()).spawn(move || {
            if let Err(e) = thumbs::trim(&cache, CACHE_LIMIT) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::debug!(error = %e, "The thumbnail cache could not be trimmed");
                }
            }
            while let Ok(req) = rx.recv() {
                let made = thumbs::load_or_make(&cache, &req.path);
                let sent = match made {
                    Ok(t) => {
                        let pixels = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&t.rgba, t.width, t.height);
                        Some(pixels)
                    }
                    Err(e) => {
                        tracing::debug!(file = %req.name, error = %e, "No thumbnail for this file");
                        None
                    }
                };
                let _ = ui.upgrade_in_event_loop(move |ui| super::arrived(&ui, req.name, req.key, sent));
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "The thumbnail worker could not start; icons keep their glyphs");
        }
        Pictures { tx: Some(tx), ready: HashMap::new(), asked: HashSet::new(), failed: HashSet::new() }
    }

    /// A model with no worker, for tests and the preview: nothing is ever asked for.
    pub fn none() -> Pictures {
        Pictures { tx: None, ready: HashMap::new(), asked: HashSet::new(), failed: HashSet::new() }
    }

    /// The thumbnail key for an item of the listing.
    pub fn key_of(dir: &std::path::Path, item: &Item) -> String {
        thumbs::key(&dir.join(&item.name), item.mtime.unwrap_or(std::time::UNIX_EPOCH), item.size)
    }

    /// The picture for `item` if it is made. Asks for it (once) if it can be cut and has not been.
    pub fn get(&mut self, dir: &std::path::Path, item: &Item) -> Option<Image> {
        thumbs::source_of(&item.name)?;
        let key = Self::key_of(dir, item);
        if let Some((have, img)) = self.ready.get(&item.name) {
            if *have == key {
                return Some(img.clone());
            }
        }
        if !self.asked.contains(&key) && !self.failed.contains(&key) {
            if let Some(tx) = &self.tx {
                self.asked.insert(key.clone());
                let _ = tx.send(Request { name: item.name.clone(), path: dir.join(&item.name), key });
            }
        }
        None
    }

    /// The worker answered. `None` means the file cannot be cut. Returns whether an icon changed.
    pub fn arrived(&mut self, name: String, key: String, pixels: Option<SharedPixelBuffer<Rgba8Pixel>>) -> bool {
        self.asked.remove(&key);
        match pixels {
            Some(p) => {
                self.ready.insert(name, (key, Image::from_rgba8(p)));
                true
            }
            None => {
                self.failed.insert(key);
                false
            }
        }
    }

    /// Forget pictures of files that are gone, so the map does not grow with every file ever seen.
    pub fn retain(&mut self, names: &HashSet<&str>) {
        self.ready.retain(|n, _| names.contains(n.as_str()));
        if self.failed.len() > 4096 {
            self.failed.clear();
        }
    }
}
