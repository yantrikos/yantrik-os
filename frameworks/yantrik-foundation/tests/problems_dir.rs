//! A process that runs under a home of its own (an app under `$YANTRIK_HOME`) files its problem
//! records there, panics included, never into the person's folder. A binary of its own: the
//! folder is set once per process.

use yantrik_foundation::problems;

#[test]
fn records_go_where_the_process_was_told_and_the_first_word_stands() {
    let home = std::env::temp_dir().join(format!("yantrik-problems-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let own = home.join("data/yantrik/problems");
    assert_ne!(problems::dir(), own);
    assert!(problems::set_dir(own.clone()));
    assert_eq!(problems::dir(), own);
    assert!(!problems::set_dir(home.join("elsewhere")), "a process has one home");
    assert_eq!(problems::dir(), own);
    let filed = problems::record_failure("yantrik-test", "settings", "the file is not this app's").expect("filed");
    assert!(filed.starts_with(&own), "{}", filed.display());
    assert_ne!(problems::default_dir(), own, "the session's own folder is not where it went");
    let _ = std::fs::remove_dir_all(&home);
}
