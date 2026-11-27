//! `set_web_search`: the one web search setting, as an agent asks for it (#1439).
//!
//! Graded `dangerous`: it decides where every web search the person's minds and tools make goes
//! (their own SearXNG, which sees every query, or the OS's own browser), so it is asked about every
//! time, in auto too, and never answered by an "Allow for this session" rule: a rule is per action
//! and not per address. The card says where searches would go (`wire::web_search::explain_set`).
//! `describe shell` shows the setting under `settings.web_search`, without the address.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::App;

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let weak = ui.as_weak();
    surface.action(
        Action::new(
            "set_web_search",
            "Choose where web searches go: `off` sends every search the person's minds and tools make to the OS's own browser (DuckDuckGo, then Google); `searxng` sends them all to the person's own SearXNG at `url`, and to nothing else. The address is tested first and saved only if a test search finds results. Asks the person every time.",
        )
        .risk("dangerous")
        .explain(|args| {
            crate::wire::web_search::explain_set(args["service"].as_str().unwrap_or_default().trim(), args["url"].as_str().unwrap_or_default())
        })
        .arg(Param::text("service").describe("`off` or `searxng`"))
        .arg(Param::text("url").optional().describe("The SearXNG's address, for `searxng`: http://192.168.1.20:8888")),
        move |args| {
            let ui = weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
            let service = args["service"].as_str().unwrap_or_default().trim().to_string();
            let url = args["url"].as_str().unwrap_or_default().to_string();
            let said = crate::wire::web_search::set_from_agent(&service, &url, &ui)?;
            Ok(serde_json::json!({ "web_search": crate::wire::web_search::describe(), "did": said }))
        },
    )
}
