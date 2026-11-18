//! `tools.golden` (#1124, round 9; Fable's decision of 10 Nov): every tool `register_all`
//! registers, as `name grade spawn-class`, so no tool ships ungraded or with a spawn nobody has
//! read. A tool that is missing from the file, added to it, or whose grade changed fails here,
//! and the failure prints the line to review. The spawn class is the reviewer's word and the
//! lint's (`xtask check-architecture`, rule `mind-spawn`) is what holds it to the doors; the
//! classes are listed in `golden.rs`, which also reads the file. `yantrik-companion` pins the tools
//! its own `build_registry` adds in the same form.
//!
//! Plugin tools come from the person's own plugin files, so they are not listed: every one of
//! them starts through `spawn::mind_command` (checked below).

use std::collections::BTreeMap;

use yantrik_companion_core::tools::ToolRegistry;

use crate::golden::{parse, problems};
use crate::PermissionLevel;

const GOLDEN: &str = include_str!("../tools.golden");

fn golden() -> BTreeMap<String, (String, String)> {
    parse("tools.golden", GOLDEN)
}

/// What the registry holds: tool name to grade.
fn registered() -> BTreeMap<String, String> {
    let mut reg = ToolRegistry::new();
    crate::register_builtin(&mut reg);
    reg.list_metadata(PermissionLevel::Dangerous)
        .into_iter()
        .map(|m| (m.name.to_string(), format!("{:?}", m.permission).to_lowercase()))
        .collect()
}

#[test]
fn tools_golden_lists_every_registered_tool_with_its_grade() {
    let problems = problems("tools.golden", &golden(), &registered());
    assert!(problems.is_empty(), "tools.golden is out of date:
{}", problems.join("
"));
}

/// A tool that says `none` starts nothing: no door and no program in a tool file that is listed
/// `none` for every tool it holds would be a cheap lie, so the file-level facts are held too.
#[test]
fn a_door_is_named_by_at_least_one_tool_that_uses_it() {
    let golden = golden();
    let has = |class: &str| golden.values().any(|(_, c)| c == class);
    // Each door the sources use is claimed by some tool in the golden.
    for (door, needle, class) in [
        ("spawn::mind_command", include_str!("coder_deps.rs"), "mind_command"),
        ("spawn::mind_command", include_str!("claude.rs"), "mind_command"),
        ("browser_spawn", include_str!("browser.rs"), "browser"),
        ("open_in_shell", include_str!("workspace.rs"), "open_app"),
        ("safe_command", include_str!("desktop.rs"), "safe_command"),
    ] {
        assert!(needle.contains(door.rsplit("::").next().unwrap()), "{door} is no longer used");
        assert!(has(class), "no tool in tools.golden is `{class}`, but the sources use {door}");
    }
}

/// Plugin tools are the person's own files; each starts through `spawn::mind_command`.
#[test]
fn plugin_tools_start_through_the_mind_door() {
    let src = include_str!("plugin.rs");
    let code = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
    assert!(code.contains("spawn::mind_command"), "plugin tools must spawn through mind_command");
    assert!(!code.contains("Command::new"), "plugin.rs must not build a command itself");
}
