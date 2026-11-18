//! What the Mind, keys and menus may ask System Monitor by name: the same three actions, graded
//! as they were, each the same command a button runs.
//!
//! `kill_process` is published as `dangerous`. Every other action here is a view change and
//! keeps no setting (#48: nothing here outlives the turn); this one ends someone's work, there
//! is no undo, and the caller's ceiling should have to allow it explicitly.

use std::time::Duration;

use serde_json::{json, Value};
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Intent, Intents, Settle};
use yantrik_app::Refused;

use crate::model::{Command, Model};
use crate::SystemMonitor;

/// How long `kill_process` keeps its caller waiting: the three seconds the dispatch gave it when
/// the kill ran inline (`UI_ROUNDTRIP`). The service's own call is bounded at two
/// (`SyncRpcClient`'s default) and the read-back of the process table at one
/// (`outcome::VERIFY_BUDGET`).
pub const KILL_WITHIN: Duration = Duration::from_secs(3);

pub fn table(table: &mut Intents<SystemMonitor>) {
    // In Private mode only the machine's load stays (#924): the processes, the filter, the disks'
    // mount points and the interfaces can say what the person is doing.
    table.shown_in_private(&["cpu_percent", "cpu_window_ms", "load_average", "memory_percent", "swap_percent"]);
    table.add(
        Intent::new(
            Action::new("sort_processes", "Order the process list by what it is using")
                .arg(Param::text("by").describe("cpu | memory")),
            |a| {
                // As given, untrimmed: the answer echoes it.
                let by = a.raw()["by"].as_str().unwrap_or_default();
                let column = match by.to_lowercase().as_str() {
                    "cpu" => 0,
                    "memory" | "mem" => 1,
                    other => {
                        return Err(Refused::new(
                            "bad_sort",
                            format!("sort by cpu or memory, not `{other}`"),
                        ))
                    }
                };
                Ok(Command::SortProcesses { column, by: by.to_string() })
            },
        )
        .reply(|m: &Model| json!({ "sorted_by": m.sorted_by })),
    );
    table.add(
        Intent::new(
            Action::new("filter_processes", "Show only processes whose name matches")
                .arg(Param::text("query").describe("Empty string clears the filter")),
            // Read raw: an empty query is the documented way to clear the filter.
            |a| Ok(Command::Search(a.raw()["query"].as_str().unwrap_or_default().to_string())),
        )
        .reply(|m: &Model| json!({ "showing": m.shown().len() })),
    );
    table.add(
        Intent::new(
            Action::new("kill_process", "End a running process by pid")
                .arg(Param::integer("pid"))
                .arg(Param::flag("force").describe("SIGKILL instead of SIGTERM").optional())
                // Everything else on this surface changes a view. This ends someone's work, and
                // there is no undo — so it must clear the caller's ceiling on its own.
                .risk("dangerous"),
            |a| {
                let pid = a.raw()["pid"]
                    .as_i64()
                    .ok_or_else(|| Refused::new("bad_pid", "`pid` must be a number"))? as i32;
                let force = a.raw()["force"].as_bool().unwrap_or(false);
                Ok(Command::End { pid, force })
            },
        )
        // The same path the End and Force Kill buttons take. It answers once the process table
        // has been read back, so a reply means the pid is gone, and a refusal names which of the
        // ways it can fail happened.
        .settles(Settle::Explicit { within: KILL_WITHIN })
        .reply(|m: &Model| m.ended.as_ref().map(|o| o.json()).unwrap_or(Value::Null)),
    );
}

#[cfg(test)]
mod grade_tests {
    //! The grades, read from this file's text: a test that would otherwise need the registry.

    fn declaration(name: &str) -> String {
        let src = include_str!("intents.rs");
        let src = src.split("#[cfg(test)]").next().unwrap_or_default();
        let quoted = format!("\"{name}\"");
        let from = src
            .find(&quoted)
            .unwrap_or_else(|| panic!("system-monitor no longer publishes {name}"));
        let rest = &src[from..];
        let end = rest.find("table.add(").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn ending_a_process_is_dangerous_and_the_views_ask_nothing() {
        assert!(
            declaration("kill_process").contains(".risk(\"dangerous\")"),
            "`system-monitor.kill_process` ends someone's work and cannot be undone"
        );
        for name in ["sort_processes", "filter_processes"] {
            let d = declaration(name);
            assert!(
                !d.contains(".risk("),
                "`system-monitor.{name}` changes a view and keeps no setting (#48): standard, \
                 as it always was. Declaration as written:\n{d}"
            );
        }
    }
}
