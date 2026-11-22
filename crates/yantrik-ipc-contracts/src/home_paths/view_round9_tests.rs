//! Round 9 of #1265: the decision on the round 8 read, the tests that belong to the view itself.

use super::tests::{home, in_view};
use super::*;
use crate::home_paths::view_env;

/// 6. Long names in the split folders: the words stop at the byte budget, the rest of the children
/// are hidden in name order, and the command is not refused.
#[test]
fn long_named_children_of_the_split_folders_are_hidden_past_a_byte_budget() {
    let (top, home) = home();
    let long = |i: usize| format!("{i:04}-{}", "p".repeat(240));
    for i in 0..400 {
        std::fs::create_dir_all(home.join("Pictures").join(long(i))).unwrap();
        std::fs::create_dir_all(home.join("Videos").join(long(i))).unwrap();
    }
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    let words = view_args(&home, &home, &[]).expect("a long name does not refuse the command").0;
    for folder in ["Pictures", "Videos"] {
        let bound: Vec<&String> = words.iter().filter(|w| Path::new(w).parent() == Some(&home.join(folder))).collect();
        assert!(!bound.is_empty() && bound.len() < 1600, "{folder}: some are bound, not all ({})", bound.len());
        // Each child is named twice (source and target) in its three words.
        let bytes: usize = bound.iter().map(|w| w.len() + 1).sum::<usize>() + bound.len() / 2 * 11;
        assert!(bytes <= MAX_CHILD_BYTES + 1024, "{folder}: {bytes} bytes of words");
        // The first in name order are the ones kept; the last are hidden.
        assert!(in_view(&words, &home, &home.join(folder).join(long(0))), "{folder}: the first child is in the view");
        assert!(!in_view(&words, &home, &home.join(folder).join(long(399))), "{folder}: the last child is hidden");
    }
    let total: usize = words.iter().map(|w| w.len() + 1).sum();
    assert!(total < 512 << 10, "the whole command line stays well under the kernel's limit: {total}");
    let _ = std::fs::remove_dir_all(top);
}

/// 8. A manifest's `env` loses the names that change where a program loads code or settings from,
/// and the person's own config keeps them.
#[test]
fn a_manifests_loader_names_are_dropped_and_the_persons_config_keeps_them() {
    let env = vec![("LD_PRELOAD".to_string(), "/x.so".to_string()), ("API_URL".to_string(), "u".to_string())];
    let (kept, dropped) = view_env::from_manifest(env.clone());
    assert_eq!(dropped, ["LD_PRELOAD"]);
    assert_eq!(kept, [("API_URL".to_string(), "u".to_string())]);
    // The person's own config goes to `allowed` as `extra`, whole.
    let mine = view_env::allowed(Vec::new(), Path::new("/home/p"), &env);
    assert!(mine.iter().any(|(k, v)| k == "LD_PRELOAD" && v == "/x.so"), "{mine:?}");
}
