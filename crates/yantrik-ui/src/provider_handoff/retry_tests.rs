//! An apply that fails or stops after it has written, and is tried again (#1205). A gateway plan mints
//! a new token each time, so the retry writes different bytes than the attempt that failed; its own
//! half-done files must not be read as the person's. One file is in the shared table
//! (rule_table_tests.rs); here are the plans with two files. Real apply and revert in a temp home.

use super::*;

fn home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("handoff-retry-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".config/yantrik")).unwrap();
    dir
}

fn offer() -> Offer {
    Offer { token: Token(yantrik_gateway::tokens::mint().unwrap()), model: "picked".into(), models: Vec::new() }
}

fn pi_files(home: &Path) -> Vec<PathBuf> {
    adapter_for("pi").unwrap().files(home)
}

fn plan(home: &Path) -> Plan {
    adapter_for("pi").unwrap().plan_gateway(home, &offer()).unwrap()
}

fn write_all(files: &[PathBuf], texts: &[&str]) {
    for (f, t) in files.iter().zip(texts) {
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t).unwrap();
    }
}

fn beside(files: &[PathBuf], infix: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for f in files {
        for e in std::fs::read_dir(f.parent().unwrap()).unwrap() {
            let p = e.unwrap().path();
            if p.file_name().unwrap().to_string_lossy().contains(infix) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn fails_after_writing(home: &Path) {
    let mut p = plan(home);
    p.mind_post = Some(Token("x".into()));
    assert!(apply(home, &p).is_err());
}

#[test]
fn a_two_file_plan_that_fails_after_the_writes_is_retried_without_a_false_copy() {
    let h = home("two-files");
    let files = pi_files(&h);
    assert!(files.len() >= 2);
    write_all(&files, &[r#"{"mine":1}"#, r#"{"mine":2}"#]);
    fails_after_writing(&h);
    assert!(marker(&h, "pi").is_none(), "nothing says it was applied");
    let m = apply(&h, &plan(&h)).unwrap();
    assert!(m.replaced_note().is_none(), "{:?}", m.replaced_note());
    assert!(beside(&files, ".yantrik-replaced-").is_empty(), "a file Yantrik wrote is not kept as the person's");
    assert_eq!(revert(&h, "pi").unwrap(), None, "no sentence claims a copy that is not there");
    assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), r#"{"mine":1}"#);
    assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), r#"{"mine":2}"#);
    assert!(beside(&files, ".before-yantrik").is_empty(), "the backups this code made are removed by its revert");
}

#[test]
fn a_two_file_plan_that_fails_on_the_second_backup_is_retried_the_same_way() {
    let h = home("second-backup");
    let files = pi_files(&h);
    write_all(&files, &[r#"{"mine":1}"#, r#"{"mine":2}"#]);
    // The second file's backup name is taken by a link: the first file is written, the second is refused.
    let blocker = backup_path(&files[1]);
    std::os::unix::fs::symlink(h.join("nowhere"), &blocker).unwrap();
    assert!(apply(&h, &plan(&h)).is_err());
    assert_ne!(std::fs::read_to_string(&files[0]).unwrap(), r#"{"mine":1}"#, "the first file was written");
    std::fs::remove_file(&blocker).unwrap();
    let m = apply(&h, &plan(&h)).unwrap();
    assert!(m.replaced_note().is_none());
    assert!(beside(&files, ".yantrik-replaced-").is_empty());
    assert_eq!(revert(&h, "pi").unwrap(), None);
    assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), r#"{"mine":1}"#);
    assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), r#"{"mine":2}"#);
    assert!(beside(&files, ".before-yantrik").is_empty());
}

#[test]
fn with_a_stray_backup_the_retry_names_the_one_copy_of_the_person_and_none_of_yantriks() {
    let h = home("stray");
    let files = pi_files(&h);
    write_all(&files, &[r#"{"mine":"edited"}"#, r#"{"mine":2}"#]);
    std::fs::write(backup_path(&files[0]), r#"{"mine":"old"}"#).unwrap();
    fails_after_writing(&h);
    let m = apply(&h, &plan(&h)).unwrap();
    let copies = beside(&files, ".yantrik-replaced-");
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(std::fs::read_to_string(&copies[0]).unwrap(), r#"{"mine":"edited"}"#);
    assert!(m.replaced_note().unwrap().contains(&copies[0].display().to_string()));
    let said = revert(&h, "pi").unwrap().unwrap();
    assert!(said.contains(&copies[0].display().to_string()), "{said}");
    assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), r#"{"mine":"old"}"#);
    assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), r#"{"mine":2}"#);
}

#[test]
fn a_revert_that_stopped_after_one_file_is_finished_by_the_next_one() {
    let h = home("revert-stops");
    let files = pi_files(&h);
    write_all(&files, &[r#"{"mine":1}"#, r#"{"mine":2}"#]);
    apply(&h, &plan(&h)).unwrap();
    // The first file was put back and the marker says so; the second revert step never ran.
    let mut m = marker(&h, "pi").unwrap();
    std::fs::rename(backup_path(&files[0]), &files[0]).unwrap();
    m.files[0].restored = true;
    std::fs::write(marker_path(&h, "pi"), serde_json::to_string(&m).unwrap()).unwrap();
    assert_eq!(revert(&h, "pi").unwrap(), None);
    assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), r#"{"mine":1}"#);
    assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), r#"{"mine":2}"#);
    assert!(marker(&h, "pi").is_none());
}
