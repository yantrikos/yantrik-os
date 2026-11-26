//! What a card's link buttons point at: a web address's host, and a file under the person's home.

/// The host of an `http` or `https` URL, lowercased, or `None` for anything else. The button
/// shows it ("Open example.com"), so a label cannot dress one site up as another.
pub fn url_host(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Credentials in a URL are how `https://bank.example@evil.example` reads as the bank.
    if authority.contains('@') {
        return None;
    }
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) { h } else { authority }
    });
    let ok = !host.is_empty()
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
    ok.then(|| host.to_string())
}

/// The path an `open_file` ref resolves to, after symlinks, when it is a regular file under
/// `home`. `None` drops the button.
///
/// Read when the notice is stored and again when the button is pressed: a file can be replaced by
/// a symlink to `/etc/shadow` in between, and only the second read is what opens.
///
/// Nothing hidden: a path with any component that starts with `.`, where it was written or where
/// it resolves, is refused. That is where a home keeps what a card must never put on the screen
/// a mind can read — `~/.ssh`, `~/.gnupg`, `~/.config/*/credentials`, the keyrings, browser
/// profiles, and this desktop's own `~/.local/share/yantrik`.
///
/// And through the rule every agent read shares (`home_paths::may_read_file`, with its
/// `PROTECTED` list), as the review asked: outside the protected places, and a file with one name,
/// because a hard link `~/Documents/key.txt` to `~/.ssh/id_ed25519` has no hidden component and
/// no link to follow, and is the key.
pub fn file_under(reference: &str, home: &std::path::Path) -> Option<std::path::PathBuf> {
    crate::home_paths::may_read_file(reference, home).ok()?;
    let home = std::fs::canonicalize(home).ok()?;
    let hidden = |p: &std::path::Path| p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.'));
    let written = std::path::Path::new(reference);
    if written.strip_prefix(&home).map_or_else(|_| hidden(written), hidden) {
        return None;
    }
    let real = std::fs::canonicalize(reference).ok()?;
    let inside = real.strip_prefix(&home).ok()?;
    if inside.as_os_str().is_empty() || hidden(inside) {
        return None;
    }
    if crate::home_paths::is_protected(&real) {
        return None;
    }
    std::fs::metadata(&real).ok()?.is_file().then_some(real)
}
