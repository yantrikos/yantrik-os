//! The sentences the documents layer says, pinned.
//!
//! Twice a Rust line continuation (`\` at the end of a line inside a string) was lost in an edit, and
//! every editor said "written and              your draft". The editors' tests only look for a word
//! of the sentence, so nothing noticed. These two tests are what notices: the default moved/deleted
//! sentences are asserted byte for byte, and no string the documents layer (or an app that overrides
//! its words) can say holds a run of spaces.

use std::path::{Path, PathBuf};

use super::files::read::inode;
use super::files::save::{stranded, SaveJob};
use super::files::FileRules;

/// The default words, as every editor but ySheets has them.
struct Defaults;

impl FileRules for Defaults {
    fn max_bytes(&self) -> usize {
        1 << 20
    }
    fn validate(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn reader(&self) -> fn(&Path) -> Result<String, String> {
        |p| std::fs::read_to_string(p).map_err(|e| e.to_string())
    }
    fn cannot_read(&self, _: &Path, e: &std::io::Error) -> String {
        e.to_string()
    }
    fn not_regular(&self) -> String {
        "not regular".into()
    }
    fn too_big(&self, _: &Path, _: u64) -> String {
        "too big".into()
    }
    fn not_utf8(&self) -> String {
        "not utf8".into()
    }
    fn cannot_open(&self, _: &Path, e: &std::io::Error) -> String {
        e.to_string()
    }
    fn relative(&self, _: &Path) -> String {
        "relative".into()
    }
    fn cannot_folder(&self, e: &std::io::Error) -> String {
        e.to_string()
    }
    fn open_elsewhere(&self, _: &str) -> String {
        "elsewhere".into()
    }
    fn already_there(&self, _: &Path) -> String {
        "there".into()
    }
    fn changed_on_disk(&self, _: &Path) -> String {
        "changed".into()
    }
    fn linked(&self, _: &Path) -> String {
        "linked".into()
    }
    fn read_only(&self, _: &Path) -> String {
        "read only".into()
    }
    fn save_failed(&self, e: &dyn std::fmt::Display) -> String {
        e.to_string()
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yantrik-sentences-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn job(target: &Path, on_disk: &str, inode: Option<super::files::read::Inode>) -> SaveJob {
    SaveJob {
        text: "new".into(),
        target: target.to_path_buf(),
        own: Some(target.to_path_buf()),
        on_disk: Some(on_disk.into()),
        unsure: None,
        inode,
        overwrite: false,
        agent: false,
        others: Vec::new(),
        elsewhere: PathBuf::new(),
    }
}

#[test]
fn the_default_moved_or_deleted_sentences_are_exactly_these_words() {
    let dir = scratch("stranded");
    let was = dir.join("was.txt");
    std::fs::write(&was, "hello").unwrap();
    let id = inode(&was);
    let now = dir.join("now.txt");
    std::fs::rename(&was, &now).unwrap();

    let found = stranded(&Defaults, &job(&was, "hello", id), &was);
    assert_eq!(
        found,
        format!(
            "{} is not there any more \u{2014} the same file looks to be at {} now. Nothing was written and your \
             draft is intact: Save As to {} to write your changes into it.",
            was.display(),
            now.display(),
            now.display()
        )
    );

    let lost = dir.join("lost.txt");
    let nowhere = stranded(&Defaults, &job(&lost, "something else entirely", None), &lost);
    assert_eq!(
        nowhere,
        format!(
            "{} was moved or deleted, so there is nothing there to write into. Nothing was written and your draft \
             is intact: Save As with overwrite=true to write it here, or Save As to wherever it went.",
            lost.display()
        )
    );
}

/// Every string literal of `source` outside comments and outside the items a `#[cfg(test)]` marks
/// (a `mod { }` block, or one `use`/`fn`/`mod x;` up to its `;`), with a line continuation (`\` and a
/// newline, and the indentation after it) taken out as Rust takes it out. The rest of the file is
/// read to its end: a test module near the top of a file hides nothing after it.
fn literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    scan(&chars, &mut i, &mut out, false);
    out
}

fn starts(chars: &[char], at: usize, text: &str) -> bool {
    text.chars().enumerate().all(|(k, c)| chars.get(at + k) == Some(&c))
}

/// Reads from `*i`. With `item`, reads one item (to its closing `}` or its `;`) and stops there;
/// without, reads to the end. A `#[cfg(test)]` met on the way has the item after it skipped.
fn scan(chars: &[char], i: &mut usize, out: &mut Vec<String>, item: bool) {
    let mut depth = 0usize;
    let mut braced = false;
    while *i < chars.len() {
        let c = chars[*i];
        if starts(chars, *i, "#[cfg(test)]") {
            *i += "#[cfg(test)]".len();
            // What follows is test code: read past it, collecting nothing.
            let mut skipped = Vec::new();
            scan(chars, i, &mut skipped, true);
            continue;
        }
        match c {
            '/' if chars.get(*i + 1) == Some(&'/') => {
                while *i < chars.len() && chars[*i] != '\n' {
                    *i += 1;
                }
            }
            '/' if chars.get(*i + 1) == Some(&'*') => {
                *i += 2;
                while *i + 1 < chars.len() && !(chars[*i] == '*' && chars[*i + 1] == '/') {
                    *i += 1;
                }
                *i += 2;
            }
            // A character literal that is a quote.
            '\'' if chars.get(*i + 1) == Some(&'"') && chars.get(*i + 2) == Some(&'\'') => *i += 3,
            '\'' if chars.get(*i + 1) == Some(&'\\') => *i += 4,
            '"' => {
                let mut text = String::new();
                *i += 1;
                while *i < chars.len() && chars[*i] != '"' {
                    if chars[*i] == '\\' {
                        match chars.get(*i + 1) {
                            Some('\n') => {
                                *i += 2;
                                while chars.get(*i).is_some_and(|c| c.is_whitespace()) {
                                    *i += 1;
                                }
                                continue;
                            }
                            Some(c) => {
                                text.push(*c);
                                *i += 2;
                                continue;
                            }
                            None => {}
                        }
                    }
                    text.push(chars[*i]);
                    *i += 1;
                }
                out.push(text);
                *i += 1;
            }
            '{' => {
                depth += 1;
                braced = true;
                *i += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                *i += 1;
                if item && braced && depth == 0 {
                    return;
                }
            }
            ';' if item && depth == 0 => {
                *i += 1;
                return;
            }
            _ => *i += 1,
        }
    }
}

fn sources_under(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if path.is_dir() && name != "tests" {
            sources_under(&path, found);
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") && !name.contains("_tests") {
            found.push(path);
        }
    }
}

/// Every source the guard reads, and what it found in each: (path, string literals scanned).
fn scanned() -> Vec<(PathBuf, Vec<String>)> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources_under(&here.join("src/documents"), &mut files);
    sources_under(&here.join("../../apps/spreadsheet/src"), &mut files);
    assert!(files.len() > 10, "the walk found {} files", files.len());
    files.into_iter().map(|f| (f.clone(), literals(&std::fs::read_to_string(&f).unwrap()))).collect()
}

/// No sentence says a run of spaces: not the documents layer's, and not an app's that overrides its
/// words (ySheets', whose source is beside this crate's).
#[test]
fn no_string_the_documents_layer_or_an_overriding_app_says_has_a_run_of_spaces() {
    let mut bad = Vec::new();
    for (file, texts) in scanned() {
        for text in texts {
            if text.contains("   ") {
                bad.push(format!("{}: {text:?}", file.display()));
            }
        }
    }
    assert!(bad.is_empty(), "a run of spaces in a sentence (a lost line continuation?):\n{}", bad.join("\n"));
}

/// The guard reads past a `#[cfg(test)]` that sits near the top of a file: sentences it has to
/// have seen, in files that put the attribute early, are among what it scanned.
#[test]
fn the_guard_reads_past_an_early_cfg_test_to_the_sentences_after_it() {
    let seen = scanned();
    let has = |file: &str, part: &str| {
        seen.iter()
            .filter(|(f, _)| f.ends_with(file))
            .any(|(_, texts)| texts.iter().any(|t| t.contains(part)))
    };
    // update/mod.rs has `#[cfg(test)]` at line 29 of 125; this sentence is at line 65.
    assert!(has("spreadsheet/src/update/mod.rs", "cannot run while ySheets is reading or writing a file"));
    // The stranded sentences, in a file with its tests at the end.
    assert!(has("documents/files/save.rs", "was moved or deleted, so there is nothing there to write into"));
    for (file, texts) in &seen {
        println!("{}: {} literals scanned", file.display(), texts.len());
    }
}

#[test]
fn the_literal_reader_takes_a_line_continuation_out_and_sees_a_lost_one() {
    let kept = "let a = \"written and \\\n    your draft\";";
    assert_eq!(literals(kept), vec!["written and your draft".to_string()]);
    let lost = "let a = \"written and              your draft\";";
    assert!(literals(lost)[0].contains("   "));
    assert_eq!(literals("// \"in a comment    \"\nlet c = '\"'; let s = \"x\";"), vec!["x".to_string()]);
    // Test code is skipped, whether a block or one item, and what comes after it is read.
    let early = "#[cfg(test)]\nmod tests {\n    fn f() { let a = \"a  {  b   c\"; }\n}\nfn g() { let s = \"after\"; }\n";
    assert_eq!(literals(early), vec!["after".to_string()]);
    let single = "#[cfg(test)]\nuse x::\"y\";\nfn g() { let s = \"after\"; }\n";
    assert_eq!(literals(single), vec!["after".to_string()]);
}
