//! #1357 round 4: private, bounded, never deleted.

use super::*;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-audit-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mode_of(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

fn gunzip(p: &Path) -> String {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::MultiGzDecoder::new(std::fs::File::open(p).unwrap()).read_to_string(&mut text).unwrap();
    text
}

#[test]
fn files_are_0600_and_the_side_directory_0700() {
    let d = dir("perm");
    let audit = d.join("mind-audit.jsonl");
    let args = json!({ "command": "k".repeat(INLINE_CHARS + 10) });
    let field = full(&args, &side_dir(&audit)).unwrap();
    append_line(&audit, "{}").unwrap();
    assert_eq!(mode_of(&audit), 0o600);
    assert_eq!(mode_of(&side_dir(&audit)), 0o700);
    assert_eq!(mode_of(&side_dir(&audit).join(field["file"].as_str().unwrap())), 0o600);
}

#[test]
fn a_side_file_is_capped_at_a_megabyte_and_what_is_past_it_is_refused() {
    let d = dir("cap");
    let ok = json!({ "command": "k".repeat(SIDE_FILE_MAX - 100) });
    assert!(full(&ok, &side_dir(&d.join("a.jsonl"))).is_ok());
    let over = json!({ "command": "k".repeat(SIDE_FILE_MAX + 1) });
    assert!(full(&over, &side_dir(&d.join("a.jsonl"))).unwrap_err().contains("in full"));
}

fn line(unix: u64, n: usize) -> String {
    json!({ "unix": unix, "app": "t", "action": "run", "n": n, "pad": "p".repeat(200) }).to_string()
}

/// The oldest month goes to a compressed archive beside the file; the line says so; nothing is lost.
#[test]
fn a_full_file_archives_its_oldest_month_and_says_so() {
    let d = dir("lines");
    let audit = d.join("mind-audit.jsonl");
    // September 2026 (x3), October 2026 (x3), then November (x6 that stay).
    let (sep, oct, nov) = (1_790_000_000u64, 1_792_700_000u64, 1_795_500_000u64);
    let mut all = Vec::new();
    for (i, t) in [sep, sep + 10, sep + 20, oct, oct + 10, oct + 20, nov, nov + 1, nov + 2, nov + 3, nov + 4, nov + 5].iter().enumerate() {
        all.push(line(*t, i));
    }
    std::fs::write(&audit, all.join("\n") + "\n").unwrap();
    maintain(&audit, 100, u64::MAX, 6);
    let live = std::fs::read_to_string(&audit).unwrap();
    assert!(!live.contains("\"n\":0,") && !live.contains("\"n\":2,"), "September left the live file");
    assert!(live.contains("\"n\":3,") && live.contains("\"n\":11,"), "October and the last lines stay");
    let note: serde_json::Value = serde_json::from_str(live.lines().last().unwrap()).unwrap();
    assert_eq!(note["archived"]["month"], "2026-09");
    assert_eq!(note["archived"]["lines"], 3);
    let name = note["archived"]["into"].as_str().unwrap();
    let archived = gunzip(&d.join(name));
    assert_eq!(archived.lines().count(), 3, "all three are in the archive, whole");
    assert_eq!(archived.lines().next().unwrap(), all[0]);
    assert_eq!(mode_of(&d.join(name)), 0o600);
    // A second round adds a member to the same kind of archive and never rewrites an old one.
    maintain(&audit, 100, u64::MAX, 6);
    assert!(std::fs::read_to_string(&audit).unwrap().contains("2026-10"), "October is next");
    assert_eq!(gunzip(&d.join(name)).lines().count(), 3, "September's archive is as it was");
}

/// Side files: past the directory's cap the oldest month is archived (whole), then removed from
/// the directory, and a line names what moved.
#[test]
fn a_full_side_directory_archives_its_oldest_month_and_says_so() {
    let d = dir("args");
    let audit = d.join("mind-audit.jsonl");
    let side = side_dir(&audit);
    let mut names = Vec::new();
    for i in 0..3 {
        let f = full(&json!({ "command": format!("{i}{}", "s".repeat(INLINE_CHARS)) }), &side).unwrap();
        names.push(f["file"].as_str().unwrap().to_string());
    }
    // All three are "now"; the cap is below their size, so this month is the oldest month.
    maintain(&audit, u64::MAX, 100, KEEP);
    assert_eq!(std::fs::read_dir(&side).unwrap().count(), 0, "moved out of the directory");
    let note: serde_json::Value = serde_json::from_str(std::fs::read_to_string(&audit).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(note["archived_args"]["files"].as_array().unwrap().len(), 3);
    let archived = gunzip(&d.join(note["archived_args"]["into"].as_str().unwrap()));
    for name in &names {
        assert!(archived.contains(name), "{name} is in the archive");
    }
    assert!(archived.contains(&"s".repeat(INLINE_CHARS)), "with its whole text");
}

/// A file an earlier version made under the umask, and its folder, are narrowed at the next open.
#[test]
fn a_0644_file_and_its_0755_folder_are_made_private_at_the_next_write() {
    let d = dir("narrow");
    let audit = d.join("mind-audit.jsonl");
    std::fs::write(&audit, "{}\n").unwrap();
    std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    let side = side_dir(&audit);
    std::fs::create_dir_all(&side).unwrap();
    std::fs::set_permissions(&side, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(mode_of(&audit), 0o644, "the starting point is the wide one");
    let entry = crate::mind_mode::record_checked("bypass", "t", &Default::default(), "narrow-test", "run", &json!({}), "sensitive", "ran").0;
    append(&entry, &json!({ "command": "k".repeat(INLINE_CHARS + 1) }), &audit).unwrap();
    assert_eq!(mode_of(&audit), 0o600);
    assert_eq!(mode_of(&d), 0o700);
    assert_eq!(mode_of(&side), 0o700);
}

/// A note is bounded and trims the file like an entry: one too large is a line that names its size
/// and hash, and a full file archives its oldest month at the next note.
#[test]
fn a_note_is_capped_and_runs_maintain() {
    let d = dir("note");
    let audit = d.join("mind-audit.jsonl");
    let big = json!({ "kind": "caller_report", "reported": "r".repeat(NOTE_MAX + 1) });
    append_note(&audit, &big);
    let last: serde_json::Value = serde_json::from_str(std::fs::read_to_string(&audit).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(last["kind"], "note_too_long");
    assert_eq!(last["sha256"], sha256(&big.to_string()));
    assert!(std::fs::metadata(&audit).unwrap().len() < NOTE_MAX as u64);
    let (sep, nov) = (1_790_000_000u64, 1_795_500_000u64);
    let old: Vec<String> = (0..4).map(|i| line(sep + i, i as usize)).collect();
    std::fs::write(&audit, old.join("\n") + "\n" + &line(nov, 9) + "\n").unwrap();
    append_note_within(&audit, &json!({ "kind": "caller_report" }), 100, u64::MAX, 2);
    let live = std::fs::read_to_string(&audit).unwrap();
    assert!(live.contains("\"archived\""), "the note's write archived the oldest month: {live}");
    assert!(std::fs::read_dir(&d).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".jsonl.gz")));
}
