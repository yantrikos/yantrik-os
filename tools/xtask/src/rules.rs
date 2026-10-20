//! The rules of docs/architecture.md, each turning the workspace into findings.

use crate::layers::{Layer, Placements};
use crate::ratchet::Finding;
use crate::workspace::Member;

/// A source file longer than this is a finding (docs/architecture.md, "File size").
pub const HARD_LINES: usize = 1200;
/// A file longer than this is counted in the report, to be split when next touched.
pub const SOFT_LINES: usize = 600;

/// Members `layers.txt` does not place. These always fail: a new crate is placed on purpose.
pub fn unplaced(members: &[Member], placements: &Placements) -> Vec<String> {
    members
        .iter()
        .filter(|m| placements.place(&m.dir).is_none())
        .map(|m| m.dir.clone())
        .collect()
}

/// `layer a->b`: a uses b, and b sits above where a may reach.
pub fn layers(members: &[Member], placements: &Placements) -> Vec<Finding> {
    let layer_of = |name: &str| {
        members
            .iter()
            .find(|m| m.name == name)
            .and_then(|m| placements.place(&m.dir))
    };
    let mut out = Vec::new();
    for m in members {
        let Some(from) = placements.place(&m.dir) else {
            continue;
        };
        for used in &m.uses {
            let Some(to) = layer_of(used) else { continue };
            if !from.may_use(to) {
                out.push(Finding::new("layer", format!("{}->{}", m.name, used)));
            }
        }
    }
    out
}

/// `build-path dir/build.rs`: an app's or a framework's build reaching into another crate's folder
/// by a relative path, where it should use that crate's library (the app screens compiled from the
/// shell's folder, step 2 and 3 of the migration).
pub fn build_paths(
    members: &[Member],
    placements: &Placements,
    read: impl Fn(&str) -> Option<String>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for m in members {
        if !placements
            .place(&m.dir)
            .is_some_and(Layer::builds_from_libraries_only)
        {
            continue;
        }
        let path = format!("{}/build.rs", m.dir);
        let Some(code) = read(&path) else { continue };
        let reaches_out = code
            .lines()
            .map(str::trim_start)
            .filter(|l| !l.starts_with("//"))
            .any(|l| l.contains("\"../"));
        if reaches_out {
            out.push(Finding::new("build-path", path));
        }
    }
    out
}

/// `size path lines` for each file over [`HARD_LINES`], and how many are over [`SOFT_LINES`].
pub fn sizes(files: &[(String, usize)]) -> (Vec<Finding>, usize) {
    let hard = files
        .iter()
        .filter(|(_, n)| *n > HARD_LINES)
        .map(|(path, n)| Finding {
            rule: "size",
            subject: path.clone(),
            measure: Some(*n),
        })
        .collect();
    let soft = files
        .iter()
        .filter(|(_, n)| *n > SOFT_LINES && *n <= HARD_LINES)
        .count();
    (hard, soft)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, dir: &str, uses: &[&str]) -> Member {
        Member {
            name: name.into(),
            dir: dir.into(),
            uses: uses.iter().map(|u| u.to_string()).collect(),
        }
    }

    fn workspace() -> (Vec<Member>, Placements) {
        let members = vec![
            member("notes", "apps/notes", &["kit", "shell-core"]),
            member("kit", "crates/kit", &[]),
            member("shell-core", "crates/shell-core", &[]),
            member("shell", "crates/shell", &["kit", "mind"]),
            member("mind", "crates/mind", &["shell-core"]),
            member("notes-tests", "tests/notes", &["shell"]),
            member("newcomer", "crates/newcomer", &["kit"]),
        ];
        let placements = Placements::parse(
            "app apps/*\nL1 crates/kit\nL3 crates/shell-core\nL4 crates/shell\nL5 crates/mind\ntool tests/*\n",
        )
        .unwrap();
        (members, placements)
    }

    #[test]
    fn what_points_up_is_found() {
        let (members, placements) = workspace();
        let found: Vec<String> = layers(&members, &placements)
            .into_iter()
            .map(|f| f.subject)
            .collect();
        assert_eq!(found, vec!["notes->shell-core", "shell->mind"]);
        assert_eq!(unplaced(&members, &placements), vec!["crates/newcomer"]);
    }

    #[test]
    fn an_app_build_that_reaches_into_another_crate_is_found() {
        let (members, placements) = workspace();
        let read = |path: &str| match path {
            "apps/notes/build.rs" => {
                Some("let p = dir.join(\"../../crates/shell/ui\");".to_string())
            }
            "crates/shell/build.rs" => Some("(\"ML\", \"../mind\")".to_string()),
            _ => None,
        };
        let found = build_paths(&members, &placements, read);
        assert_eq!(
            found,
            vec![Finding::new("build-path", "apps/notes/build.rs")],
            "the shell's own build is not held to it"
        );
        let commented = |_: &str| Some("// once: \"../old\"\nfn main() {}".to_string());
        assert!(build_paths(&members, &placements, commented).is_empty());
    }

    #[test]
    fn long_files_are_found_and_counted() {
        let files = vec![
            ("a.rs".to_string(), 1201),
            ("b.slint".to_string(), 1200),
            ("c.rs".to_string(), 601),
            ("d.rs".to_string(), 10),
        ];
        let (hard, soft) = sizes(&files);
        assert_eq!(
            hard,
            vec![Finding {
                rule: "size",
                subject: "a.rs".into(),
                measure: Some(1201)
            }]
        );
        assert_eq!(soft, 2);
    }
}
