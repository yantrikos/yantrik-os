//! What the pool used, per day, provider and model: for "today: 412 calls across 6 providers,
//! $0 spent", and so a restart does not forget the day's counts. Kept as one small JSON file,
//! written whole and replaced atomically, holding the last 14 days.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    pub requests: u64,
    pub tokens: u64,
    /// Refusals: 429s and errors.
    pub refused: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// "2026-09-30" → "groq|openai/gpt-oss-120b" → totals.
    days: BTreeMap<String, BTreeMap<String, Totals>>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl Ledger {
    /// The ledger at `path`, or an empty one when there is none yet or it cannot be read.
    pub fn open(path: &Path) -> Ledger {
        use std::io::Read;
        // Fourteen days of a few providers is a few kilobytes; anything near a megabyte is not
        // a ledger, and is not read whole into memory to find that out.
        let mut bytes = Vec::new();
        let read = std::fs::File::open(path).and_then(|f| f.take(1 << 20).read_to_end(&mut bytes));
        let mut l: Ledger = read.ok().and_then(|_| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        l.path = Some(path.to_path_buf());
        l.trim();
        l
    }

    fn trim(&mut self) {
        while self.days.len() > 14 {
            let oldest = self.days.keys().next().cloned();
            if let Some(k) = oldest {
                self.days.remove(&k);
            }
        }
    }

    pub fn add(&mut self, day: &str, provider: &str, model: &str, ok: bool, tokens: u64) {
        let t = self.days.entry(day.to_string()).or_default().entry(format!("{provider}|{model}")).or_default();
        t.requests = t.requests.saturating_add(1);
        t.tokens = t.tokens.saturating_add(tokens);
        if !ok {
            t.refused = t.refused.saturating_add(1);
        }
        self.trim();
    }

    /// One day's totals per provider (models summed).
    pub fn day(&self, day: &str) -> BTreeMap<String, Totals> {
        let mut out: BTreeMap<String, Totals> = BTreeMap::new();
        for (k, t) in self.days.get(day).into_iter().flatten() {
            let provider = k.split('|').next().unwrap_or(k).to_string();
            let o = out.entry(provider).or_default();
            o.requests = o.requests.saturating_add(t.requests);
            o.tokens = o.tokens.saturating_add(t.tokens);
            o.refused = o.refused.saturating_add(t.refused);
        }
        out
    }

    /// Write it out: to a file beside it (never followed if it is a link, readable by its owner
    /// only), then renamed over it. Not synced: it is a count of use, written on every call with
    /// the pool's lock held, and losing the last few on a power cut costs nothing.
    pub fn save(&self) -> std::io::Result<()> {
        use std::io::Write;
        let Some(path) = &self.path else { return Ok(()) };
        let tmp = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        drop(file);
        std::fs::rename(&tmp, path)
    }
}

/// "2026-09-30" for a unix time, in UTC.
pub fn day_name(now: i64) -> String {
    let (y, m, d) = super::quota::civil(now.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}
