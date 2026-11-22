//! Where the audit of unasked actions is kept, who can read it, how large it may grow, and what
//! happens to it when it is full (#1353, round 4 of #1357).
//!
//! - **Private.** The file is created `0600` and the side directory `0700`, never wider: a
//!   command can carry a secret.
//! - **Whole.** A line holds the arguments inline to [`INLINE_CHARS`] with their SHA-256; longer
//!   ones are stored whole in a side file under `mind-audit-args/`, named by the hash. A side file
//!   holds at most [`SIDE_FILE_MAX`] bytes: arguments past that cannot be written down in full, so
//!   the action that carried them is not run.
//! - **Bounded, and never forgotten.** When the file passes [`FILE_MAX`] or the side directory
//!   passes [`DIR_MAX`], the oldest month is moved into a compressed archive beside the file
//!   (`mind-audit-YYYY-MM.jsonl.gz`, `mind-audit-args-YYYY-MM.jsonl.gz`; each write is a gzip
//!   member, so an archive only ever grows). Nothing is deleted, and a line is appended that says
//!   what was archived and where.

use crate::mind_mode::AuditEntry;
use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The most characters of arguments kept inline on an audit line.
pub const INLINE_CHARS: usize = 4000;
/// The most bytes one side file holds.
pub const SIDE_FILE_MAX: usize = 1024 * 1024;
/// How large the audit file may get before its oldest month is archived.
pub const FILE_MAX: u64 = 2 * 1024 * 1024;
/// How large the side directory may get before its oldest month is archived.
pub const DIR_MAX: u64 = 64 * 1024 * 1024;
/// Lines that stay in the live file whatever the month they are from.
pub const KEEP: usize = 400;

/// One writer at a time: the file, its trim and the side directory are changed together.
static WRITING: Mutex<()> = Mutex::new(());

/// The side directory beside the audit file.
pub fn side_dir(audit: &Path) -> PathBuf {
    audit.with_file_name("mind-audit-args")
}

pub fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    // A directory made by an earlier version, or by a umask, is narrowed too.
    if dir.file_name().is_some_and(|n| n == "mind-audit-args") {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn private_file(path: &Path, append: bool) -> std::io::Result<std::fs::File> {
    let mut open = std::fs::OpenOptions::new();
    open.create(true).mode(0o600);
    if append { open.append(true) } else { open.write(true).truncate(true) };
    open.open(path)
}

/// The `args_full` field of an audit line for `args`: the exact text and its hash, and where the
/// text is kept when it is too long to sit on the line. `Err` when it cannot be kept whole.
pub fn full(args: &serde_json::Value, dir: &Path) -> Result<serde_json::Value, String> {
    let text = args.to_string();
    let sha = sha256(&text);
    if text.chars().count() <= INLINE_CHARS {
        return Ok(serde_json::json!({ "sha256": sha, "text": text }));
    }
    if text.len() > SIDE_FILE_MAX {
        return Err(format!("its arguments are {} bytes and the record keeps at most {SIDE_FILE_MAX}, so it could not be written down in full", text.len()));
    }
    let name = format!("{sha}.json");
    let kept = private_dir(dir).and_then(|_| private_file(&dir.join(&name), false)).and_then(|mut f| f.write_all(text.as_bytes()));
    if let Err(e) = kept {
        tracing::warn!(error = %e, "could not store the whole arguments of an unasked action");
        // Never less than the whole: with no file to point at, the line carries it all.
        return Ok(serde_json::json!({ "sha256": sha, "text": text, "file_failed": true }));
    }
    Ok(serde_json::json!({ "sha256": sha, "file": name, "chars": text.chars().count() }))
}

/// Append one line to `path`, `0600`, synced.
pub fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut file = private_file(path, true)?;
    writeln!(file, "{line}")?;
    file.sync_all()
}

/// Write one entry with its whole arguments, then keep the files within their bounds.
pub fn append(entry: &AuditEntry, args: &serde_json::Value, audit: &Path) -> Result<(), String> {
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = audit.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).map_err(|e| format!("the audit directory could not be made ({e})"))?;
    }
    let mut line = entry.to_json();
    line["args_full"] = full(args, &side_dir(audit))?;
    append_line(audit, &line.to_string()).map_err(|e| format!("the audit line could not be written ({e})"))?;
    maintain(audit, FILE_MAX, DIR_MAX, KEEP);
    Ok(())
}

/// Append a line that is not an entry: a caller's report, or a note of what was archived.
pub fn append_note(audit: &Path, note: &serde_json::Value) {
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if let Err(e) = append_line(audit, &note.to_string()) {
        tracing::warn!(error = %e, "could not write a note to the mind audit");
    }
}

fn month_of(unix: u64) -> String {
    chrono::DateTime::from_timestamp(unix as i64, 0).map(|t| t.format("%Y-%m").to_string()).unwrap_or_else(|| "unknown".into())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Add `lines` to an archive as one more gzip member.
fn archive_into(path: &Path, lines: &[String]) -> std::io::Result<()> {
    let mut gz = GzEncoder::new(private_file(path, true)?, Compression::default());
    for line in lines {
        writeln!(gz, "{line}")?;
    }
    gz.finish()?.sync_all()
}

/// Archive the oldest month when the file or the side directory is over its bound. A failure
/// leaves everything where it was: the archive is written before anything is moved out.
pub fn maintain(audit: &Path, file_max: u64, dir_max: u64, keep: usize) {
    if let Err(e) = archive_lines(audit, file_max, keep).and_then(|_| archive_args(audit, dir_max)) {
        tracing::warn!(error = %e, "could not archive the oldest month of the mind audit");
    }
}

fn archive_lines(audit: &Path, file_max: u64, keep: usize) -> std::io::Result<()> {
    if std::fs::metadata(audit).map(|m| m.len() <= file_max).unwrap_or(true) {
        return Ok(());
    }
    let body = std::fs::read_to_string(audit)?;
    let lines: Vec<&str> = body.lines().collect();
    let eligible = lines.len().saturating_sub(keep);
    let mut month = String::new();
    let months: Vec<String> = lines.iter().map(|l| {
        if let Some(u) = serde_json::from_str::<serde_json::Value>(l).ok().and_then(|v| v.get("unix").and_then(|u| u.as_u64())) {
            month = month_of(u);
        }
        month.clone()
    }).collect();
    // A note about an earlier archive is not a month's record: it stays.
    let is_note = |l: &str| l.starts_with("{\"archived");
    let Some(oldest) = months[..eligible].iter().zip(&lines).filter(|(m, l)| !m.is_empty() && !is_note(l)).map(|(m, _)| m.clone()).min() else { return Ok(()) };
    let (moved, stay): (Vec<_>, Vec<_>) = lines.iter().enumerate().partition(|(i, l)| *i < eligible && months[*i] == oldest && !is_note(l));
    if moved.is_empty() {
        return Ok(());
    }
    let archive = audit.with_file_name(format!("mind-audit-{oldest}.jsonl.gz"));
    archive_into(&archive, &moved.iter().map(|(_, l)| l.to_string()).collect::<Vec<_>>())?;
    let note = serde_json::json!({ "archived": { "month": oldest, "into": archive.file_name().and_then(|n| n.to_str()), "lines": moved.len(), "unix": now() } });
    let mut rest: Vec<String> = stay.iter().map(|(_, l)| l.to_string()).collect();
    rest.push(note.to_string());
    let temp = audit.with_extension("jsonl.new");
    private_file(&temp, false)?.write_all((rest.join("\n") + "\n").as_bytes())?;
    std::fs::rename(&temp, audit)
}

fn archive_args(audit: &Path, dir_max: u64) -> std::io::Result<()> {
    let dir = side_dir(audit);
    let Ok(read) = std::fs::read_dir(&dir) else { return Ok(()) };
    let files: Vec<(String, u64, u64)> = read
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let at = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some((e.file_name().to_str()?.to_string(), at, m.len()))
        })
        .collect();
    if files.iter().map(|f| f.2).sum::<u64>() <= dir_max {
        return Ok(());
    }
    let Some(oldest) = files.iter().map(|f| month_of(f.1)).min() else { return Ok(()) };
    let moved: Vec<&(String, u64, u64)> = files.iter().filter(|f| month_of(f.1) == oldest).collect();
    let mut records = Vec::new();
    for (name, _, _) in &moved {
        records.push(serde_json::json!({ "file": name, "text": std::fs::read_to_string(dir.join(name))? }).to_string());
    }
    let archive = audit.with_file_name(format!("mind-audit-args-{oldest}.jsonl.gz"));
    archive_into(&archive, &records)?;
    let names: Vec<&String> = moved.iter().map(|f| &f.0).collect();
    append_line(audit, &serde_json::json!({ "archived_args": { "month": oldest, "into": archive.file_name().and_then(|n| n.to_str()), "files": names, "unix": now() } }).to_string())?;
    // They are in the archive now; only then do they leave the directory.
    for (name, _, _) in &moved {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "audit_store_tests.rs"]
mod tests;
