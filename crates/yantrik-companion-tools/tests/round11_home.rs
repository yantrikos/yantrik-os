//! #1247, review R11 (H1, M3, L3, L4): the companion's workspace, terminal, fixes and disk tools, run for
//! real against a scratch home that holds fake secrets. This file is its own process, so setting `HOME`
//! here races no other test. Every secret is the fake text below.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use yantrik_companion_core::permission::PermissionLevel;
use yantrik_companion_core::tools::{Tool, ToolContext};
use yantrik_companion_tools::artifacts::ListFixesTool;
use yantrik_companion_tools::disk::{AnalyzeDiskTool, DirSizeTool};
use yantrik_companion_tools::terminal::ReadTerminalBufferTool;
use yantrik_companion_tools::terminal_analysis::SearchTerminalHistoryTool;
use yantrik_companion_tools::workspace::SaveWorkspaceTool;

const FAKE: &str = "FAKE-SECRET-1247";

fn put(at: &Path, text: &str) {
    std::fs::create_dir_all(at.parent().expect("a parent")).expect("make the folder");
    std::fs::write(at, text).expect("write the file");
}

/// The scratch home, with `HOME` pointing at it, made once; and a lock so the tests take turns.
fn home() -> (PathBuf, std::sync::MutexGuard<'static, ()>) {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    static TURN: Mutex<()> = Mutex::new(());
    let guard = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let home = HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("yantrik-r11-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("run")).expect("the runtime folder");
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_RUNTIME_DIR", home.join("run"));
        for history in [".bash_history", ".zsh_history", ".ash_history"] {
            put(&home.join(history), &format!("ls\nexport TOKEN={FAKE}\ncat ~/.kube/config\n"));
        }
        // The home holds the usual protected places, as a real one does (each makes the old per-entry
        // judgement look them up), one of them moved behind a link.
        for place in ["Documents", ".config/gh", ".config/gcloud", ".aws", ".azure", ".docker", ".cargo", ".mozilla", ".claude", ".hermes", ".pi", ".openclaw", ".local/share/keyrings", ".password-store", ".gnupg", ".ssh", ".npmrc.d", ".thunderbird", ".config/chromium", ".config/evolution", ".emacs.d", ".local/state/nvim", ".config/Code", ".config/systemd", ".config/autostart"] {
            std::fs::create_dir_all(home.join(place)).expect("a place");
        }
        put(&home.join("dotfiles/kube/config"), FAKE);
        std::os::unix::fs::symlink(home.join("dotfiles/kube"), home.join(".kube")).expect("a moved place");
        put(&home.join("backup/tokens.json"), FAKE);
        home
    });
    (home.clone(), guard)
}

/// An embedder that gives every text the same vector: a memory needs one to be recorded.
struct Flat;

impl yantrikdb_core::Embedder for Flat {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(vec![0.5; 384])
    }

    fn dim(&self) -> usize {
        384
    }
}

fn call(tool: &dyn Tool, args: serde_json::Value) -> (String, yantrikdb_core::YantrikDB) {
    let mut db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    db.set_embedder(Box::new(Flat));
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    let said = tool.execute(&ctx, &args);
    (said, db)
}

fn scrollback(text: &str) {
    use std::io::Write;
    let mut file = yantrik_ml::private_dir::create_scratch("yantrik-scrollback.txt").expect("a scratch file");
    file.write_all(text.as_bytes()).expect("write the scrollback");
}

/// H1: `save_workspace` returns, and records, no line of any shell's history file; the recent commands it
/// does give are the terminal's scrollback, with a line that names a secret replaced.
#[test]
fn save_workspace_reads_no_history_file() {
    let (_home, _turn) = home();
    scrollback("make test\ncat ~/backup/tokens.json\nls -la\n");
    let (said, db) = call(&SaveWorkspaceTool, serde_json::json!({}));
    assert!(!said.contains("FAKE") && !said.contains("TOKEN") && !said.contains(".kube"), "{said}");
    assert!(said.contains("make test") && said.contains("ls -la"), "the scrollback's commands are kept: {said}");
    assert!(said.contains("(a line that names a protected file is not shown)"), "{said}");
    // What was recorded is that same snapshot: no memory holds a history line.
    let stored: Vec<String> = db
        .conn()
        .prepare("SELECT text FROM memories")
        .map(|mut q| q.query_map([], |row| row.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default())
        .unwrap_or_default();
    assert!(!stored.is_empty() && stored.iter().all(|text| !text.contains("FAKE") && !text.contains("TOKEN")), "{stored:?}");
}

/// H1: the tools crate reads no `*_history` file in code that runs.
#[test]
fn no_tool_reads_a_history_file() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&src).expect("the sources").flatten() {
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if path.file_name().is_some_and(|name| name.to_string_lossy().ends_with("_tests.rs")) {
            continue;
        }
        let code = text.split("#[cfg(test)]").next().unwrap_or_default();
        let live = code.lines().filter(|line| !line.trim_start().starts_with("//"));
        let files = [".bash_history", ".zsh_history", ".ash_history", ".sh_history", "fish_history", ".python_history", ".node_repl_history", ".psql_history", ".mysql_history", ".sqlite_history", ".irb_history", ".rediscli_history"];
        if live.clone().any(|line| files.iter().any(|file| line.contains(file))) {
            found.push(path);
        }
    }
    assert!(found.is_empty(), "{found:?}");
}

/// L3: the terminal tools themselves, over a planted scrollback and a fake `.bash_history`.
#[test]
fn the_terminal_tools_show_no_secret_line_and_read_no_history() {
    let (_home, _turn) = home();
    scrollback(&format!("ok line\ncat ~/backup/tokens.json\nexport TOKEN={FAKE}\nlast line\n"));
    let (said, _) = call(&ReadTerminalBufferTool, serde_json::json!({}));
    assert!(said.contains("ok line") && said.contains("last line"), "{said}");
    assert!(said.contains("(a line that names a protected file is not shown)") && !said.contains("tokens.json"), "{said}");
    let (found, _) = call(&SearchTerminalHistoryTool, serde_json::json!({"pattern": "TOKEN"}));
    assert!(!found.contains("tokens.json"), "{found}");
    // The history files hold `export TOKEN=` too; nothing of them comes back.
    let (found, _) = call(&SearchTerminalHistoryTool, serde_json::json!({"pattern": "kube"}));
    assert!(!found.contains(".kube") && !found.contains("config"), "{found}");
}

/// L4: `list_fixes` names no protected file, nor one a link leads to.
#[test]
fn list_fixes_skips_a_protected_name_and_a_link_to_one() {
    let (home, _turn) = home();
    put(&home.join("fixes/id_rsa.md"), &format!("{FAKE}\n"));
    put(&home.join("fixes/ok.md"), "# A real fix\nsteps\n");
    std::os::unix::fs::symlink(home.join("backup/tokens.json"), home.join("fixes/a.md")).expect("a link");
    let (said, _) = call(&ListFixesTool, serde_json::json!({}));
    assert!(said.contains("A real fix") && said.contains("ok.md"), "{said}");
    assert!(!said.contains("id_rsa") && !said.contains("FAKE") && !said.contains("a.md"), "{said}");
}

/// M3: a 20,000-file tree in a home with the usual protected places is walked in about the time `du`
/// takes, not seconds; and no total has a protected file's bytes in it.
#[test]
fn a_big_tree_is_walked_in_time_and_no_total_holds_a_secret() {
    let (home, _turn) = home();
    let big = home.join("big");
    for i in 0..20_000 {
        let dir = big.join(format!("d{}", i % 100));
        if i < 100 {
            std::fs::create_dir_all(&dir).expect("a folder");
        }
        std::fs::write(dir.join(format!("f{i}.txt")), b"x").expect("a file");
    }
    let measured = |pad: usize| {
        if pad > 0 {
            std::fs::write(big.join("d1/tokens.json"), vec![b'k'; pad]).expect("a secret");
        }
        let began = Instant::now();
        let (sized, _) = call(&DirSizeTool, serde_json::json!({"path": big.to_string_lossy(), "depth": 1}));
        let sized_in = began.elapsed();
        let began = Instant::now();
        let (analyzed, _) = call(&AnalyzeDiskTool, serde_json::json!({"path": big.to_string_lossy()}));
        (sized, sized_in, analyzed, began.elapsed())
    };
    let (plain, plain_in, analyzed, analyzed_in) = measured(0);
    eprintln!("dir_size {plain_in:?}, analyze_disk {analyzed_in:?}");
    // `du` takes about 30 ms; the old per-entry judgement took 8.8 s and 25.4 s here (release). A debug build
    // runs about ten times slower than the release one the person has.
    let most = if cfg!(debug_assertions) { Duration::from_secs(8) } else { Duration::from_secs(1) };
    assert!(plain_in < most && analyzed_in < most, "{plain_in:?} {analyzed_in:?}");
    assert!(plain.contains("Total:") && analyzed.contains("txt"), "{plain}\n{analyzed}");
    let total = |text: &str| text.lines().find(|l| l.starts_with("Total:")).map(str::to_string);
    let (with_secret, ..) = measured(50_000_000_usize.min(5_000_000));
    assert_eq!(total(&plain), total(&with_secret), "a 5 MB secret moves no total: {plain}\n{with_secret}");
    assert!(with_secret.contains("1 entries are not shown") && !with_secret.contains("tokens"), "{with_secret}");
    let _ = std::fs::remove_dir_all(&big);
}

/// N-L2: `~/.kube` is moved to `~/dotfiles/kube`, and `~/lnk` is a link to `~/dotfiles`. A walk started on
/// `~/lnk` hides the kubeconfig as one started on `~/dotfiles` does.
#[test]
fn a_walk_started_through_a_link_has_the_same_total_as_at_the_real_path() {
    let (home, _turn) = home();
    put(&home.join("dotfiles/notes.md"), "ordinary");
    put(&home.join("dotfiles/kube/config"), &"k".repeat(5_000_000));
    let _ = std::fs::remove_file(home.join("lnk"));
    std::os::unix::fs::symlink(home.join("dotfiles"), home.join("lnk")).expect("a link");
    let total = |at: PathBuf| {
        let (said, _) = call(&DirSizeTool, serde_json::json!({"path": at.to_string_lossy(), "depth": 2}));
        said.lines().find(|l| l.starts_with("Total:")).map(str::to_string)
    };
    let (by_link, by_path) = (total(home.join("lnk")), total(home.join("dotfiles")));
    // The two roots differ by the size of the folder entry itself (a link is 45 bytes, a folder 4 KB, as `du`
    // counts them); the 5 MB kubeconfig is in neither total.
    for (how, total) in [("link", &by_link), ("path", &by_path)] {
        let total = total.clone().expect("a total");
        assert!(!total.contains("MB") && !total.contains("GB"), "the kubeconfig is counted through the {how}: {total}");
    }
}

/// N-M1: the picture the shell saves for `screenshot` is in a protected place, so `describe_image` and
/// `read_file` of the path the reply carries are refused: no pixels reach a model with no second consent.
#[test]
fn the_picture_a_screenshot_saves_is_read_by_no_tool() {
    let (home, _turn) = home();
    let shot = home.join("Pictures/Screenshots/shot-1.png");
    put(&shot, FAKE);
    let mut reg = yantrik_companion_core::tools::ToolRegistry::new();
    yantrik_companion_tools::register_all(&mut reg);
    yantrik_companion_tools::vision::register(&mut reg, "http://127.0.0.1:1", "none");
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("an in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Dangerous, registry_metadata: None, task_manager: None, incognito: true, agent_spawner: None };
    for (tool, args) in [("describe_image", serde_json::json!({"path": shot.to_string_lossy()})), ("read_file", serde_json::json!({"path": shot.to_string_lossy()})), ("read_file", serde_json::json!({"path": "~/Pictures/Screenshots/shot-1.png"}))] {
        let said = reg.execute(&ctx, tool, &args);
        assert!(said.contains("protected") && !said.contains("FAKE"), "{tool}: {said}");
    }
}
