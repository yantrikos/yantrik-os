//! Choosing a file: the desktop's own file picker, over the FileChooser portal.
//!
//! The machine has one file picker, the one the browser's Save and Upload open:
//! `org.freedesktop.portal.FileChooser`, which xdg-desktop-portal-gtk supplies (#33, #379, held
//! by `release-check`). This asks it, from a worker thread, and waits for the person's answer on
//! the request's `Response` signal. When there is no portal to ask — a session without D-Bus, or
//! a machine from before #379 — the caller is told so, in words, and falls back to typing a path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zbus::zvariant::{OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const OBJECT: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Chosen(PathBuf),
    Cancelled,
    /// No picker could be asked, and why.
    Unavailable(String),
}

/// Ask the person for a CSV file to open.
pub fn open_file() -> Choice {
    ask("OpenFile", "Open a sheet", None).unwrap_or_else(Choice::Unavailable)
}

/// Ask the person where to save, suggesting `current` (its folder and its name).
pub fn save_file(current: Option<&Path>) -> Choice {
    ask("SaveFile", "Save the sheet as CSV", current).unwrap_or_else(Choice::Unavailable)
}

fn ask(method: &str, title: &str, current: Option<&Path>) -> Result<Choice, String> {
    let unavailable = |e: zbus::Error| format!("the file picker could not be asked ({e})");
    let conn = zbus::blocking::Connection::session().map_err(unavailable)?;
    let token = format!("ysheets_{}_{}", std::process::id(), next_token());
    let sender = conn
        .unique_name()
        .ok_or("the file picker could not be asked (no name on the session bus)")?
        .trim_start_matches(':')
        .replace('.', "_");
    // Subscribe to the answer before asking, at the path the portal will answer on, so a quick
    // answer cannot arrive before anyone is listening.
    let request = zbus::blocking::Proxy::new(
        &conn,
        DESTINATION,
        format!("{OBJECT}/request/{sender}/{token}"),
        "org.freedesktop.portal.Request",
    )
    .map_err(unavailable)?;
    let mut responses = request.receive_signal("Response").map_err(unavailable)?;

    let chooser = zbus::blocking::Proxy::new(&conn, DESTINATION, OBJECT, "org.freedesktop.portal.FileChooser")
        .map_err(unavailable)?;
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("modal", Value::from(true));
    let filters = vec![
        ("CSV files".to_string(), vec![(0u32, "*.csv".to_string()), (1u32, "text/csv".to_string())]),
        ("All files".to_string(), vec![(0u32, "*".to_string())]),
    ];
    options.insert("filters", Value::from(filters));
    if let Some(path) = current {
        if let Some(name) = path.file_name() {
            options.insert("current_name", Value::from(name.to_string_lossy().into_owned()));
        }
        if let Some(dir) = path.parent() {
            let mut bytes = dir.as_os_str().as_encoded_bytes().to_vec();
            bytes.push(0);
            options.insert("current_folder", Value::from(bytes));
        }
    } else if method == "SaveFile" {
        options.insert("current_name", Value::from("sheet.csv"));
    }
    let _handle: zbus::zvariant::OwnedObjectPath =
        chooser.call(method, &("", title, options)).map_err(unavailable)?;

    let answer = responses.next().ok_or("the file picker closed without answering")?;
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        answer.body().deserialize().map_err(|e| format!("the file picker's answer could not be read ({e})"))?;
    match code {
        0 => {
            let uris: Vec<String> = results
                .get("uris")
                .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default();
            match uris.first().and_then(|u| path_of_uri(u)) {
                Some(p) => Ok(Choice::Chosen(p)),
                None => Err("the file picker answered with no local file".into()),
            }
        }
        1 => Ok(Choice::Cancelled),
        _ => Err("the file picker ended without a choice".into()),
    }
}

fn next_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// `file:///home/p/My%20Sheet.csv` → `/home/p/My Sheet.csv`. Anything not a local file is `None`.
pub fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://localhost/x` and `file:///x` are both local.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(out).ok().map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_is_a_path_with_its_escapes_undone() {
        assert_eq!(path_of_uri("file:///home/p/My%20Sheet.csv"), Some(PathBuf::from("/home/p/My Sheet.csv")));
        assert_eq!(path_of_uri("file://localhost/tmp/a.csv"), Some(PathBuf::from("/tmp/a.csv")));
        assert_eq!(path_of_uri("file:///tmp/%E2%82%AC.csv"), Some(PathBuf::from("/tmp/€.csv")));
        assert_eq!(path_of_uri("https://example.com/a.csv"), None);
        assert_eq!(path_of_uri("file://server/share/a.csv"), None);
        assert_eq!(path_of_uri("file:///bad%2"), None);
    }
}
