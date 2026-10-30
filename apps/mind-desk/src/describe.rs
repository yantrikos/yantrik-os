//! What a Mind may read of the desk through its control surface: the state label, the tool by name,
//! the counts, the apps, the safety line and the model, from the same chrome the window is drawn
//! from. Never what a Mind said it is doing: that is not on the desk, and so not here.

use yantrik_app::control::View;

use crate::chrome;
use crate::model::Model;

pub fn view(m: &Model) -> View {
    let top = chrome::top(m);
    let bottom = chrome::bottom(m);
    View::new(format!("Mind desk — {}", top.label))
        .with("label", top.label)
        .with("stop_offered", top.can_stop)
        .with("tool_heading", top.strip_heading)
        .with("tool", top.strip_tool)
        .with("tool_mark", top.strip_mark)
        .with("counts", top.strip_stats)
        .with("apps", bottom.apps)
        .with("safety", bottom.safety)
        .with("model", bottom.model)
}
