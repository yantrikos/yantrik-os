//! #1247, review R11-M4: the pure parts of the MCP path rule. The proxy tool itself, run against a fake
//! server, is `tests/round11_secret_tools.rs`.

use super::*;

#[test]
fn a_string_that_reads_as_a_protected_path_is_refused_at_any_depth() {
    let home = Path::new("/home/ann");
    for args in [
        serde_json::json!({"path": "~/backup/tokens.json"}),
        serde_json::json!({"path": "/home/ann/.kube/config"}),
        serde_json::json!({"files": ["~/docs/notes.md", {"inner": "file:///home/ann/.hermes.yantrik-moved/config.yaml"}]}),
        serde_json::json!({"path": "tokens (copy 1).json"}),
        serde_json::json!({"path": "~/.kube.bak/config"}),
    ] {
        assert!(names_a_protected_path(&args, home), "{args}");
    }
    for args in [
        serde_json::json!({"path": "~/docs/notes.md", "text": "tokens are mentioned in prose", "n": 3}),
        serde_json::json!({"path": "/home/ann/.kube-tools/readme.md"}),
        serde_json::json!({}),
    ] {
        assert!(!names_a_protected_path(&args, home), "{args}");
    }
}

#[test]
fn a_listing_loses_every_line_that_names_a_secret_and_keeps_the_rest() {
    let out = without_protected_entries("[DIR] docs\n[FILE] tokens.json\n- ~/.kube/config\n  server.pem  \nnotes.md\nThe tokens.json file is named in a sentence.");
    assert_eq!(out.lines().filter(|l| !l.contains("not shown")).collect::<Vec<_>>(), ["[DIR] docs", "notes.md"]);
    assert!(out.contains("4 entries are not shown"), "{out}");
    assert_eq!(without_protected_entries("a\nb"), "a\nb", "no note when nothing was left out");
}

#[test]
fn a_sized_listing_a_copy_with_spaces_and_a_json_tree_are_filtered() {
    let sized = without_protected_entries("[FILE] tokens.json                 1.20 KB\n[FILE] notes.md                 3.00 KB\n[FILE] tokens (1).json 1 KB\n[FILE] tokens (conflicted copy 2026-11-15 101530).json 1 KB");
    assert!(!sized.contains("tokens") && sized.contains("notes.md") && sized.contains("3 entries"), "{sized}");
    let tree = without_protected_entries(r#"[{"name":"backup","type":"directory","children":[{"name":"tokens.json","type":"file"},{"name":"notes.md","type":"file"}]}]"#);
    assert!(!tree.contains("tokens") && tree.contains("notes.md") && tree.contains("1 entries"), "{tree}");
}
