//! What a Mind may read of the desk through its control surface: the same words the desk
//! draws, from the same chrome. It is the desk's own view of Mind View, and it carries nothing
//! that is not already on the window.

use yantrik_app::control::View;

use crate::chrome;
use crate::model::Model;

pub fn view(m: &Model) -> View {
    let top = chrome::top(m);
    let bottom = chrome::bottom(m);
    let summary = if top.headline.is_empty() { format!("Mind desk — {}", top.label) } else { format!("Mind desk — {}: {}", top.label, top.headline) };
    View::new(summary)
        .with("label", top.label)
        .with("headline", top.headline)
        .with("quoted", top.quoted)
        .with("reported", top.reported)
        .with("stop_offered", top.can_stop)
        .with("tool_heading", top.strip_heading)
        .with("tool", top.strip_tool)
        .with("tool_mark", top.strip_mark)
        .with("counts", top.strip_stats)
        .with("apps", bottom.apps)
        .with("safety", bottom.safety)
        .with("model", bottom.model)
}
