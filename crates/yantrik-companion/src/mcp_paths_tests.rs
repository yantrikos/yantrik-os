//! #1247, review R11-M4: the pure part of the MCP path rule. The proxy tool itself, run against a fake
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
