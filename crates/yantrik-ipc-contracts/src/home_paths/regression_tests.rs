//! #1247: no later rule may reopen a hole an earlier round closed, or close a decoy it left open.

use std::path::Path;

use super::*;

/// Every name and path asserted by a test of this change, with what it must be (see the file).
const ROUNDS: &str = include_str!("regression_names.txt");

#[test]
fn every_name_any_round_refused_is_still_refused() {
    let mut seen = [0usize; 4];
    for line in ROUNDS.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let (kind, subject) = line.split_once('\t').expect("a kind, a tab and a name");
        match kind {
            "secret" => {
                seen[0] += 1;
                assert!(is_secret_name(subject), "a credential that became readable: {subject}");
                assert!(is_protected(Path::new(&format!("/home/ann/backup/{subject}"))), "a credential path that became readable: {subject}");
            }
            "readable" => {
                seen[1] += 1;
                assert!(!is_secret_name(subject), "a decoy that became secret: {subject}");
                assert!(!is_protected(Path::new(&format!("/home/ann/Documents/{subject}"))), "a decoy path that became protected: {subject}");
            }
            "protected" => {
                seen[2] += 1;
                assert!(is_protected(Path::new(subject)), "a protected path that became readable: {subject}");
            }
            "open" => {
                seen[3] += 1;
                assert!(!is_protected(Path::new(subject)), "a decoy path that became protected: {subject}");
            }
            other => panic!("unknown kind {other}"),
        }
    }
    assert!(seen.iter().all(|&n| n > 10), "every list is read: {seen:?}");
}
