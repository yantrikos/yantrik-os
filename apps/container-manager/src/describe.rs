//! The Mind's view of Containers: a projection of the committed model, in the same keys and words
//! the app has always answered, so `yos check containers` reads the same.
//!
//! The companion has docker tools of its own, and they shell out exactly as this app does. What
//! this adds is the *user's* view: which containers they are looking at, which tab, what the log
//! pane is showing.

use serde_json::{json, Value};
use yantrik_app::control::View;

use crate::model::{Model, TABS};
use crate::runtime::Availability;
use crate::verdict::last_chars;

/// The most images and volumes a describe lists.
const LISTED: usize = 30;
/// The most of an open log a describe carries.
const LOG_TAIL_CHARS: usize = 4000;

pub fn view(m: &Model) -> View {
    let containers: Vec<Value> = m
        .containers
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "id": c.id,
                "image": c.image,
                "status": c.state,
                "detail": c.status_text,
                "ports": c.ports,
            })
        })
        .collect();
    let images: Vec<Value> = m
        .images
        .iter()
        .take(LISTED)
        .map(|i| json!({ "tag": i.repo_tag, "size": i.size_text }))
        .collect();
    let volumes: Vec<Value> = m
        .volumes
        .iter()
        .take(LISTED)
        .map(|v| json!({ "name": v.name, "driver": v.driver }))
        .collect();

    let runtime = &m.runtime;
    // Three summaries where there used to be two, and the missing one was the important one:
    // "no docker containers" was what this app said on a machine with no docker on it, which
    // is a statement about the machine that happened not to be true.
    let summary = match &m.health {
        Availability::Missing => format!("Containers — {runtime} is not installed on this machine"),
        Availability::Unreachable(reason) => {
            format!("Containers — the {runtime} daemon is not reachable: {reason}")
        }
        Availability::Ready if containers.is_empty() => {
            format!("Containers — {runtime} is running and has no containers")
        }
        Availability::Ready => format!(
            "Containers — {} running of {} on {runtime}, {} images",
            m.running(),
            m.total(),
            images.len()
        ),
    };

    let mut view = View::new(summary)
        .with("runtime", runtime.clone())
        .with("runtime_available", m.health.is_ready())
        .with("runtime_state", m.health.state_name())
        // Said twice: the strip under the header is the person's half of this.
        .with("notice", m.notice.clone())
        .with("tab", TABS[m.tab.min(TABS.len() - 1)])
        .with("running", m.running() as i32)
        .with("stopped", m.stopped() as i32)
        .with("total", m.total() as i32)
        .with("containers", Value::Array(containers))
        .with("images", Value::Array(images))
        .with("volumes", Value::Array(volumes));

    // Only when the pane is actually open. A log tail is the largest thing this surface can
    // carry, and sending one nobody asked for would make every describe expensive.
    if m.logs.open {
        view = view.with(
            "open_logs",
            json!({
                "container": m.logs.container,
                "tail": last_chars(&m.logs.text, LOG_TAIL_CHARS),
            }),
        );
    }
    view
}
