//! The format of a `tools.golden` file (#1124, round 11): one `name grade spawn-class` line per
//! tool, `#` for comments. Both golden files (this crate's, for `register_all`, and
//! `yantrik-companion`'s, for what `build_registry` adds) are read and compared here, so the two
//! tests cannot drift apart. The tests that call it live with the registries they pin.
//!
//! The spawn class is the reviewer's word, and `xtask check-architecture` (rule `mind-spawn`) holds
//! the code to the doors:
//!
//! - `none`: starts no program;
//! - `fixed`: starts fixed utilities by a string literal (or a `// fixed-program:` list);
//! - `safe_command`: starts allowlisted utilities through `safe_command`, with protected paths;
//! - `mind_command`: starts a program a Mind steers, in the outside view (`spawn::mind_command`);
//! - `browser`: starts a browser in the browser view (`browser_spawn`);
//! - `open_app`: asks the shell to start an app (`open_app`, or `start_app` for a framework app).

use std::collections::BTreeMap;

pub const CLASSES: [&str; 6] = ["none", "fixed", "safe_command", "mind_command", "browser", "open_app"];

/// Tool name to (grade, spawn class). Panics, naming `file:line`, on a malformed line.
pub fn parse(file: &str, text: &str) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for (n, line) in text.lines().enumerate().map(|(i, l)| (i + 1, l.trim())) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let [name, grade, class] = words[..] else { panic!("{file}:{n}: want `name grade spawn-class`, got `{line}`") };
        assert!(["safe", "standard", "sensitive", "dangerous"].contains(&grade), "{file}:{n}: grade `{grade}`");
        assert!(CLASSES.contains(&class), "{file}:{n}: spawn class `{class}` is not one of {CLASSES:?}");
        assert!(out.insert(name.to_string(), (grade.to_string(), class.to_string())).is_none(), "{file}:{n}: `{name}` twice");
    }
    out
}

/// What is out of date: a registered tool the golden lacks, one it lists that is not registered,
/// and a regraded tool. `registered` maps a tool name to its grade; empty when the golden holds.
pub fn problems(file: &str, golden: &BTreeMap<String, (String, String)>, registered: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for (name, grade) in registered {
        match golden.get(name) {
            None => out.push(format!("not in {file} (add it, with the spawn class you read): {name} {grade} ???")),
            Some((was, _)) if was != grade => out.push(format!("grade changed: {name} was {was}, is {grade} (review it, then change the line)")),
            Some(_) => {}
        }
    }
    for name in golden.keys().filter(|n| !registered.contains_key(*n)) {
        out.push(format!("in {file} but not registered (remove the line): {name}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(tools: &[(&str, &str)]) -> BTreeMap<String, String> {
        tools.iter().map(|(n, g)| (n.to_string(), g.to_string())).collect()
    }

    #[test]
    fn a_missing_an_extra_and_a_regraded_tool_are_each_a_problem() {
        let golden = parse("g", "# c\na safe none\nb standard fixed\nc dangerous mind_command\n");
        assert!(problems("g", &golden, &registered(&[("a", "safe"), ("b", "standard"), ("c", "dangerous")])).is_empty());
        let p = problems("g", &golden, &registered(&[("a", "safe"), ("b", "sensitive"), ("d", "safe")]));
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p.iter().any(|l| l.contains("grade changed: b")));
        assert!(p.iter().any(|l| l.contains("not in g") && l.contains("d safe")));
        assert!(p.iter().any(|l| l.contains("remove the line): c")));
    }

    #[test]
    #[should_panic(expected = "g:1: spawn class `maybe`")]
    fn an_unknown_spawn_class_is_refused() {
        parse("g", "a safe maybe\n");
    }
}
