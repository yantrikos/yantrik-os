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

/// The path an `open_summary` ref names, when it is exactly `summary.md`, a regular file (not a
/// link), directly in a folder that is directly in `<home>/Documents/Prepared`, with no hidden
/// folder and no link on the way. The same shape checks as the card's Open folder. `None` refuses.
pub fn summary_under(reference: &str, home: &std::path::Path) -> Option<std::path::PathBuf> {
    if reference.contains("..") {
        return None;
    }
    let file = std::path::Path::new(reference);
    if file.file_name()?.to_str()? != "summary.md" {
        return None;
    }
    let folder = file.parent()?;
    let root = home.join("Documents").join("Prepared");
    if folder.parent()? != root.as_path() || folder.file_name()?.to_str()?.starts_with('.') {
        return None;
    }
    let plain = |p: &std::path::Path, dir: bool| {
        std::fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink() && if dir { m.is_dir() } else { m.is_file() })
    };
    (plain(&root, true) && plain(folder, true) && plain(file, false)).then(|| file.to_path_buf())
}

#[cfg(test)]
mod summary_tests {
    use super::summary_under;

    #[test]
    fn only_summary_md_in_a_real_prepared_folder_opens() {
        let home = std::env::temp_dir().join(format!("summary-under-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let folder = home.join("Documents/Prepared/2026-11-27-x");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("summary.md"), "s").unwrap();
        std::fs::write(folder.join("script.md"), "s").unwrap();
        std::fs::create_dir_all(home.join("Documents/Prepared/.discarded")).unwrap();
        std::fs::write(home.join("Documents/Prepared/.discarded/summary.md"), "s").unwrap();
        std::fs::create_dir_all(home.join("Documents/Prepared/2026-11-27-x/deeper")).unwrap();
        std::fs::write(folder.join("deeper/summary.md"), "s").unwrap();
        let ok = |p: std::path::PathBuf| summary_under(p.to_str().unwrap(), &home).is_some();
        assert!(ok(folder.join("summary.md")));
        assert!(!ok(folder.join("script.md")), "only summary.md");
        assert!(!ok(folder.join("deeper/summary.md")), "directly in the folder");
        assert!(!ok(home.join("Documents/Prepared/.discarded/summary.md")), "not hidden");
        assert!(!ok(home.join("Documents/summary.md")), "not outside Prepared");
        assert!(!ok(folder.join("nope/../summary.md")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", home.join("Documents/Prepared/2026-11-27-x/link-target")).ok();
            let l = home.join("Documents/Prepared/2026-11-28-y");
            std::fs::create_dir_all(&l).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", l.join("summary.md")).unwrap();
            assert!(!ok(l.join("summary.md")), "a link is refused");
            std::os::unix::fs::symlink(&folder, home.join("Documents/Prepared/2026-11-29-z")).unwrap();
            assert!(!ok(home.join("Documents/Prepared/2026-11-29-z/summary.md")), "a linked folder is refused");
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
