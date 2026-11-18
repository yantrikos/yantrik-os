//! The baseline: what broke a rule on the day the rule landed. It may only shrink. A finding not in
//! it fails the check; an entry that no longer occurs fails too, until the baseline is rewritten,
//! so a fix cannot quietly be undone later.

use std::collections::BTreeMap;

/// One thing that breaks a rule: `rule` and its `subject` name it, `measure` (a line count) is
/// reported but not compared.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub rule: &'static str,
    pub subject: String,
    pub measure: Option<usize>,
}

impl Finding {
    pub fn new(rule: &'static str, subject: impl Into<String>) -> Finding {
        Finding {
            rule,
            subject: subject.into(),
            measure: None,
        }
    }

    fn key(&self) -> (String, String) {
        (self.rule.to_string(), self.subject.clone())
    }
}

pub type Baseline = BTreeMap<(String, String), Option<usize>>;

/// The baseline file: `rule subject [measure]` per line, `#` for comments.
pub fn parse(text: &str) -> Result<Baseline, String> {
    let mut out = Baseline::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let measure = match words.as_slice() {
            [_, _] => None,
            [_, _, m] => Some(
                m.parse()
                    .map_err(|_| format!("baseline line {}: `{m}` is not a count", n + 1))?,
            ),
            _ => {
                return Err(format!(
                    "baseline line {}: expected `rule subject [count]`, got `{line}`",
                    n + 1
                ))
            }
        };
        out.insert((words[0].to_string(), words[1].to_string()), measure);
    }
    Ok(out)
}

pub fn render(findings: &[Finding]) -> String {
    let mut text = String::from(
        "# What broke the architecture rules when each rule landed (tools/xtask, docs/architecture.md).\n\
         # It may only shrink: fix one, then rewrite this with `cargo run -q -p xtask -- check-architecture --write`.\n\
         # rule subject [lines]\n",
    );
    let mut sorted = findings.to_vec();
    sorted.sort();
    for f in sorted {
        match f.measure {
            Some(m) => text.push_str(&format!("{} {} {m}\n", f.rule, f.subject)),
            None => text.push_str(&format!("{} {}\n", f.rule, f.subject)),
        }
    }
    text
}

/// How today's findings stand against the baseline.
#[derive(Debug, Default)]
pub struct Verdict {
    /// Not in the baseline: these fail the check.
    pub new: Vec<Finding>,
    /// In the baseline but gone: the baseline must come down to keep the fix.
    pub fixed: Vec<(String, String)>,
    /// Still there, with a measure that grew (reported, not failed, while the rules are new).
    pub grew: Vec<(Finding, usize)>,
    /// A file that moved and carried its break with it, as (old path, new path): neither new nor
    /// fixed. The baseline is rewritten to the new path.
    pub moved: Vec<(String, String)>,
    /// Still there.
    pub kept: usize,
}

impl Verdict {
    pub fn passes(&self) -> bool {
        self.new.is_empty() && self.fixed.is_empty()
    }
}

/// Whether a new finding is a baseline entry's file under a new path: the same rule and measure,
/// the same file name, and the old path gone from the tree. Each test closes a way to spend a fix
/// on admitting an unrelated file: a shrunk file is still there, and a padded new file does not
/// share the old one's name.
fn is_move(
    rule: &str,
    old: &str,
    new: &str,
    was: Option<usize>,
    now: Option<usize>,
    gone: &dyn Fn(&str) -> bool,
) -> bool {
    let name = |p: &str| {
        std::path::Path::new(p)
            .file_name()
            .map(|n| n.to_os_string())
    };
    rule == "size"
        && was.is_some()
        && was == now
        && name(old).is_some()
        && name(old) == name(new)
        && old != new
        && gone(old)
}

/// `gone(path)` says whether a path from the baseline is no longer in the tree.
pub fn judge(findings: &[Finding], baseline: &Baseline, gone: &dyn Fn(&str) -> bool) -> Verdict {
    let mut v = Verdict::default();
    for f in findings {
        match baseline.get(&f.key()) {
            None => v.new.push(f.clone()),
            Some(was) => {
                v.kept += 1;
                if let (Some(now), Some(was)) = (f.measure, *was) {
                    if now > was {
                        v.grew.push((f.clone(), was));
                    }
                }
            }
        }
    }
    for key in baseline.keys() {
        if !findings.iter().any(|f| &f.key() == key) {
            v.fixed.push(key.clone());
        }
    }
    // A move: a new sized entry and a fixed one that `is_move` pairs. The file went somewhere
    // else, unchanged, and its break went with it (a crate moved into frameworks/ is not new slop).
    let mut i = 0;
    while i < v.new.len() {
        let f = &v.new[i];
        let twin = v.fixed.iter().position(|(rule, old)| {
            *rule == f.rule
                && is_move(
                    rule,
                    old,
                    &f.subject,
                    baseline
                        .get(&(rule.clone(), old.clone()))
                        .copied()
                        .flatten(),
                    f.measure,
                    gone,
                )
        });
        match twin {
            Some(j) => {
                let (_, old) = v.fixed.remove(j);
                let f = v.new.remove(i);
                v.moved.push((old, f.subject));
                v.kept += 1;
            }
            None => i += 1,
        }
    }
    v
}

/// Entries in `now` that `before` does not have: what was added to the baseline, by `--write` on
/// a first run or by hand. A changed line count is not an addition, and nor is a file that moved
/// (`is_move`, against an entry gone from `before`).
pub fn added(
    before: &Baseline,
    now: &Baseline,
    gone: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    let mut left: Vec<(&(String, String), &Option<usize>)> = before
        .iter()
        .filter(|(k, _)| !now.contains_key(*k))
        .collect();
    let mut out = Vec::new();
    for (key, measure) in now.iter().filter(|(k, _)| !before.contains_key(*k)) {
        let twin = left.iter().position(|(k, was)| {
            k.0 == key.0 && is_move(&k.0, &k.1, &key.1, **was, *measure, gone)
        });
        match twin {
            Some(j) => {
                left.remove(j);
            }
            None => out.push(key.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The old path is gone from the tree.
    fn gone(_: &str) -> bool {
        true
    }

    /// Every old path is still in the tree.
    fn present(_: &str) -> bool {
        false
    }

    #[test]
    fn a_file_that_moved_carries_its_break_with_it() {
        let base = parse("size crates/old/big.rs 2066\nsize y.rs 1300\n").unwrap();
        let today = [sized("frameworks/new/big.rs", 2066), sized("y.rs", 1300)];
        let v = judge(&today, &base, &gone);
        assert!(v.passes(), "new {:?} fixed {:?}", v.new, v.fixed);
        assert_eq!(
            v.moved,
            vec![(
                "crates/old/big.rs".to_string(),
                "frameworks/new/big.rs".to_string()
            )]
        );
        assert_eq!(v.kept, 2);
        // A different size is not a move: the file changed, and the break is judged afresh.
        let v = judge(
            &[sized("frameworks/new/big.rs", 2070), sized("y.rs", 1300)],
            &base,
            &gone,
        );
        assert!(!v.passes());
        assert!(v.moved.is_empty());
        // The same for the baseline held to its base.
        let now = parse("size frameworks/new/big.rs 2066\nsize y.rs 1300\n").unwrap();
        assert!(
            added(&base, &now, &gone).is_empty(),
            "a moved entry is not an addition"
        );
        let now = parse("size frameworks/new/big.rs 2070\nsize y.rs 1300\n").unwrap();
        assert_eq!(added(&base, &now, &gone).len(), 1);
    }

    /// A fix cannot be spent on admitting an unrelated file: shrinking `a.rs` below the limit
    /// and adding a new file whose line count matches `a.rs`'s baseline is two findings, not a
    /// move. The old file is still in the tree, and the new one does not carry its name.
    #[test]
    fn a_shrunk_file_and_a_new_one_of_the_same_count_are_not_a_move() {
        let base = parse("size apps/a.rs 1500\n").unwrap();
        let today = [sized("services/evil/src/new.rs", 1500)];
        // a.rs is still there (shrunk); new.rs is not its name.
        let v = judge(&today, &base, &present);
        assert!(!v.passes());
        assert!(v.moved.is_empty());
        assert_eq!(v.new.len(), 1);
        assert_eq!(v.fixed.len(), 1);
        // Even if a.rs were deleted, a file of another name is not it.
        let v = judge(&today, &base, &gone);
        assert!(!v.passes() && v.moved.is_empty());
        // And a file of the same name whose old path is still in the tree is a copy, not a move.
        let v = judge(&[sized("services/evil/src/a.rs", 1500)], &base, &present);
        assert!(!v.passes() && v.moved.is_empty());
        // The baseline check says the same.
        let now = parse("size services/evil/src/new.rs 1500\n").unwrap();
        assert_eq!(added(&base, &now, &gone).len(), 1);
        let now = parse("size services/evil/src/a.rs 1500\n").unwrap();
        assert_eq!(added(&base, &now, &present).len(), 1);
        assert!(added(&base, &now, &gone).is_empty(), "the real move");
    }

    #[test]
    fn a_line_added_to_the_baseline_is_found() {
        let before = parse("layer a->b\nsize x.rs 1500\n").unwrap();
        let now = parse("layer a->b\nsize x.rs 1700\nsize y.rs 1300\n").unwrap();
        assert_eq!(
            added(&before, &now, &gone),
            vec![("size".to_string(), "y.rs".to_string())]
        );
        assert!(
            added(&now, &before, &gone).is_empty(),
            "a shrink adds nothing"
        );
    }

    fn sized(subject: &str, lines: usize) -> Finding {
        Finding {
            rule: "size",
            subject: subject.into(),
            measure: Some(lines),
        }
    }

    #[test]
    fn the_baseline_may_only_shrink() {
        let base = parse(&render(&[
            Finding::new("layer", "a->b"),
            sized("x.rs", 1500),
            sized("y.rs", 1300),
        ]))
        .unwrap();
        assert_eq!(base.len(), 3);
        let today = [
            Finding::new("layer", "a->b"),
            sized("x.rs", 1600),
            Finding::new("layer", "c->d"),
        ];
        let v = judge(&today, &base, &gone);
        assert_eq!(
            v.new,
            vec![Finding::new("layer", "c->d")],
            "a new break fails"
        );
        assert_eq!(
            v.fixed,
            vec![("size".to_string(), "y.rs".to_string())],
            "a fix must be kept"
        );
        assert_eq!(v.grew.len(), 1, "growth is reported");
        assert_eq!(v.kept, 2);
        assert!(!v.passes());
        assert!(judge(
            &[
                Finding::new("layer", "a->b"),
                sized("x.rs", 1),
                sized("y.rs", 1)
            ],
            &base,
            &gone
        )
        .passes());
    }

    #[test]
    fn a_malformed_line_is_an_error() {
        assert!(parse("size x.rs many").is_err());
        assert!(parse("layer").is_err());
        assert!(parse("# only a comment\n\n").unwrap().is_empty());
    }
}
