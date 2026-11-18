use super::*;

/// Which rows the window shows, in the order it shows them.
///
/// Kept out of the window because it is the one piece of display logic with edge cases worth
/// asserting — a filter and a search box that disagree, a sort over a size nobody reported — and a
/// function taking a slice can be tested where one reading Slint properties cannot.
///
/// `filter` is the tab: 0 all, 1 active, 2 completed, 3 failed. `sort` is 0 date, 1 name, 2 size,
/// 3 status, matching the numbers the UI already sends.
pub fn visible_rows(items: &[Download], filter: i32, query: &str, sort: i32) -> Vec<Download> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<Download> = items
        .iter()
        .filter(|d| match filter {
            1 => d.status.is_active() || d.status == Status::Paused,
            2 => d.status == Status::Completed,
            // The "Failed" tab is where a person goes to find what went wrong, and a file that
            // finished and then vanished belongs there rather than nowhere.
            3 => d.status == Status::Failed || d.status == Status::Missing,
            _ => true,
        })
        .filter(|d| {
            query.is_empty()
                || d.filename.to_lowercase().contains(&query)
                || d.url.to_lowercase().contains(&query)
        })
        .cloned()
        .collect();

    match sort {
        1 => rows.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase())),
        // Largest first: the reason to sort a download list by size is to find the big one.
        2 => rows.sort_by(|a, b| b.total.unwrap_or(b.downloaded).cmp(&a.total.unwrap_or(a.downloaded))),
        3 => rows.sort_by(|a, b| a.status.as_str().cmp(b.status.as_str())),
        // Newest first, which is what a downloads list is for.
        _ => rows.sort_by(|a, b| b.id.cmp(&a.id)),
    }
    rows
}

/// SHA-256 of a file, streamed so the size of the file does not become the size of a buffer.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Where a download goes when nobody says: `~/Downloads`, the same place every other desktop
/// puts it, so a file an agent fetched is where the person would look for it.
pub fn default_download_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join("Downloads")
}

/// Open options that refuse a path whose last part is a link (`O_NOFOLLOW`).
pub(super) fn no_follow(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
}

/// `~/x` means what it does in a shell; anything else is taken as given.
pub fn expand_home(dir: &str) -> PathBuf {
    if let Some(rest) = dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\")) {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Path::new(&home).join(rest)
    } else if dir == "~" {
        PathBuf::from(
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string()),
        )
    } else {
        PathBuf::from(dir)
    }
}

/// The name to save a URL under.
///
/// The last non-empty path segment, minus query and fragment, with anything that is not obviously
/// safe in a filename replaced. A URL that ends in a slash, or carries a name made entirely of
/// separators, gets `download` rather than an empty name or a dotfile.
pub fn filename_from_url(url: &str) -> String {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let path = without_scheme.split(['?', '#']).next().unwrap_or("");
    // Skip the host: a bare `https://example.com` must not save as `example.com`.
    let last = path.split('/').skip(1).filter(|s| !s.is_empty()).last().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '(' | ')' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned
    }
}

/// A name that is not already taken in `dir`, by the `name (2).ext` convention.
///
/// Downloading over a file the person already has would be the one irreversible thing this app
/// could do by accident, so it does not.
pub fn unique_in(dir: &Path, name: &str) -> String {
    // `symlink_metadata`, not `exists`: a link that points at nothing is not a free name. Taken
    // as one, the transfer would write through it to wherever it points.
    let taken = |n: &str| dir.join(n).symlink_metadata().is_ok();
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        // A leading dot is the whole name of a dotfile, not a separator.
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = format!("{stem} ({n}){extension}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{stem} ({}){extension}", std::process::id())
}

/// `1.4 KB`, `45.2 MB` — one decimal, because two is noise at these magnitudes.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `45s`, `2m 30s`, `1h 05m` — as long as it is useful and no longer.
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub(super) fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `one` or `many` by count. A person reads "1 download is" and "6 downloads are", never
/// "file(s)": the hedge makes them do the grammar the app already knew.
pub fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}
