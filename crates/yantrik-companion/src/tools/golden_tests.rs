//! `tools.golden` of `yantrik-companion` (#1124, round 11): the tools `build_registry` adds on top
//! of `yantrik_companion_tools::register_all`, as `name grade spawn-class` (the format and the
//! classes are `yantrik_companion_tools::golden`'s; that crate's own golden pins the rest). The
//! reviewer found `run_background` here, a Mind-steered shell nobody had listed, because only the
//! sibling crate's tools were pinned. A tool that is added, missing or regraded fails the test.

use std::collections::BTreeMap;

use yantrik_companion_core::tools::ToolRegistry;
use yantrik_companion_tools::golden::{parse, problems};

use super::{build_registry, PermissionLevel};
use crate::config::CompanionConfig;

const GOLDEN: &str = include_str!("../../tools.golden");

fn grades(reg: &ToolRegistry) -> BTreeMap<String, String> {
    reg.list_metadata(PermissionLevel::Dangerous)
        .into_iter()
        .map(|m| (m.name.to_string(), format!("{:?}", m.permission).to_lowercase()))
        .collect()
}

/// What `build_registry` registers beyond `register_all`. The channel tools are registered only
/// when their account is configured, so they are added here as `build_registry` would.
fn registered() -> BTreeMap<String, String> {
    let mut reg = build_registry(&CompanionConfig::default());
    super::telegram::register(&mut reg, "token", "chat");
    super::whatsapp::register(&mut reg, "phone", "token", "recipient");
    super::email::register(&mut reg, Vec::new());
    let mut shared = ToolRegistry::new();
    yantrik_companion_tools::register_all(&mut shared);
    let shared = grades(&shared);
    grades(&reg).into_iter().filter(|(name, _)| !shared.contains_key(name)).collect()
}

#[test]
fn the_companions_own_tools_are_pinned_with_their_grade_and_spawn_class() {
    let problems = problems("yantrik-companion/tools.golden", &parse("tools.golden", GOLDEN), &registered());
    assert!(problems.is_empty(), "yantrik-companion/tools.golden is out of date:\n{}", problems.join("\n"));
}

/// `run_background` is the Mind's shell line: it is listed as such, and the door it starts through
/// is the one the task manager names.
#[test]
fn run_background_is_pinned_as_a_mind_command() {
    let golden = parse("tools.golden", GOLDEN);
    assert_eq!(golden.get("run_background").map(|(g, c)| (g.as_str(), c.as_str())), Some(("sensitive", "mind_command")));
    assert!(include_str!("../task_manager.rs").contains("spawn::mind_command"));
}
