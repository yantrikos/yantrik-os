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
    /// Still there.
    pub kept: usize,
}

impl Verdict {
    pub fn passes(&self) -> bool {
        self.new.is_empty() && self.fixed.is_empty()
    }
}

pub fn judge(findings: &[Finding], baseline: &Baseline) -> Verdict {
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
    v
}

/// Entries in `now` that `before` does not have: what was added to the baseline, by `--write` on
/// a first run or by hand. A changed line count is not an addition.
pub fn added(before: &Baseline, now: &Baseline) -> Vec<(String, String)> {
    now.keys()
        .filter(|k| !before.contains_key(*k))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_added_to_the_baseline_is_found() {
        let before = parse("layer a->b\nsize x.rs 1500\n").unwrap();
        let now = parse("layer a->b\nsize x.rs 1700\nsize y.rs 1300\n").unwrap();
        assert_eq!(
            added(&before, &now),
            vec![("size".to_string(), "y.rs".to_string())]
        );
        assert!(added(&now, &before).is_empty(), "a shrink adds nothing");
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
        let v = judge(&today, &base);
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
            &base
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
