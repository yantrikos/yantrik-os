//! What the Mind remembers about the document, for the rail.
//!
//! Memory is a search, not a generation: it answers from the shell even when no model does, so
//! only a missing shell takes it away. The query is the document's title, asked a beat after the
//! title stops changing (the rail refreshes by the same rule), through the shared relevance floor
//! (see `recall_relevant` for why that floor exists). The rows are added to the rail's context
//! when they arrive; a title that has moved on since drops them.
//!
//! Nothing is asked in Private mode, and nothing for a window drawn for a Mind.

use std::time::Duration;

use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::mind::{Reach, RELEVANCE_FLOOR};
use yantrik_app::rail::ContextItem;
use yantrik_app::timers::TimerId;
use yantrik_ipc_transport::privacy;

use crate::command::Command;
use crate::model::Model;
use crate::YDoc;

/// The beat the title must stand still for.
const TIMER: TimerId = TimerId("ydoc.memory");
const BEAT: Duration = Duration::from_millis(400);
/// How many memories the rail shows.
const WANT: usize = 3;

/// After every command: the title the rows are wanted for, and a timer to ask once it has stood
/// still. A title that is gone, or a Mind that is not there, drops the rows.
pub fn watch(m: &mut Model, fx: &mut Effects<YDoc>) {
    let wanted = if m.for_mind || m.text().trim().is_empty() || m.rail.reach() == Reach::NoShell {
        String::new()
    } else {
        m.title()
    };
    if wanted == m.memory_wanted {
        return;
    }
    m.memory_wanted = wanted;
    if m.memory_wanted.trim().is_empty() {
        m.memory.clear();
        m.memory_for.clear();
        return;
    }
    fx.after(TIMER, BEAT, Command::MemoryDue);
}

/// The title stood still: ask the Mind from a job.
pub fn due(m: &mut Model, fx: &mut Effects<YDoc>) {
    let query = m.memory_wanted.clone();
    if query.trim().is_empty() || query == m.memory_for {
        return;
    }
    let mind = m.mind.clone();
    fx.spawn_keyed(
        JobKey { feature: "memory", scope: Scope::App },
        Concurrency::ReplacePrevious,
        move |_| {
            // The Mind sees nothing of this desktop while the person has Private mode on.
            let found = if privacy::is_private() { Vec::new() } else { mind.recall_relevant(&query, RELEVANCE_FLOOR, WANT) };
            let rows = found
                .into_iter()
                .map(|r| ContextItem {
                    id: format!("memory:{}", r.rid),
                    label: r.text.lines().next().unwrap_or("").trim().to_string(),
                    detail: format!("{}% match", (r.score * 100.0).round() as i64),
                    source: "memory".into(),
                })
                .collect();
            Command::Remembered { query, rows }
        },
    );
}

/// The Mind answered: the rows are kept if the title is still the one they were asked for.
pub fn remembered(m: &mut Model, query: String, rows: Vec<ContextItem>) {
    if query != m.memory_wanted {
        return;
    }
    m.memory_for = query;
    m.memory = rows;
}
