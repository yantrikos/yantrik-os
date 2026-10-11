//! `show_workroom`: the Agents workroom on the person's screen, and the control surface's half of
//! the workroom's navigation (Workroom · Needs you · History, and a mind's own page).
//!
//! What a pointer can do on the workroom, a mind can ask for, with the grades the acts have:
//!
//! | on the screen | on this surface | grade |
//! |---|---|---|
//! | Start work, Start | `new_agent`, `hand_off` (a role from the catalog) | sensitive |
//! | View desk, a request's action | `show_agent` | safe |
//! | Stop… and its confirmation | `stop_agent` | standard |
//! | Chat | `use_harness` (which mind answers), `open_lens` | sensitive, safe |
//! | the navigation, History | `show_workroom` | safe |
//! | what the screen says | `describe shell`, under `workroom` and `agents` | — |
//!
//! `show_workroom` is `safe`, as `show_screen` and `show_agent` are: it changes which page is
//! drawn and nothing about any agent. It brings the shell forward, so a card waiting on the
//! person holds it (`card_watch::hold_windows`) like every act that raises a window: a mind cannot
//! move the person's view while a decision is waiting to be seen.

use serde_json::{json, Value};
use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::{AgentsState, App};

/// The pages the workroom's navigation has.
pub const PAGES: [&str; 3] = ["workroom", "needs_you", "history"];

/// What a call asked for: a page, and a mind to narrow the workroom to.
#[derive(Debug, PartialEq)]
struct Asked {
    page: &'static str,
    mind: Option<String>,
}

fn spec() -> Action {
    Action::new(
        "show_workroom",
        "Put the Agents workroom on the person's screen: `workroom` (the decisions waiting, the desks \
         at work), `needs_you` (every request waiting, oldest first) or `history` (finished runs). \
         With `mind`, the workroom shows that mind's desks and requests only. Changes nothing about \
         any agent; `show_agent` opens one run. `describe shell` reads it back under `workroom`.",
    )
    .risk("safe")
    .arg(Param::text("page").optional().describe("workroom (the default), needs_you or history"))
    .arg(Param::text("mind").optional().describe(
        "Narrow the workroom to one mind: its id as `describe shell` lists under `workroom.minds`. \
         Only with page `workroom`",
    ))
}

/// The page and the mind a call asked for, refused when either is not one there is.
fn asked(args: &Value, minds: &[(String, String)]) -> Result<Asked, String> {
    let page = args.get("page").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).unwrap_or("workroom");
    let page = PAGES
        .into_iter()
        .find(|p| *p == page)
        .ok_or_else(|| format!("`page` is `{page}`; the workroom's pages are {}.", PAGES.join(", ")))?;
    let mind = args.get("mind").and_then(Value::as_str).map(str::trim).filter(|m| !m.is_empty());
    let Some(mind) = mind else { return Ok(Asked { page, mind: None }) };
    if page != "workroom" {
        return Err(format!("`mind` narrows the workroom page only; `{page}` lists every mind's."));
    }
    if !minds.iter().any(|(id, _)| id == mind) {
        let known: Vec<&str> = minds.iter().map(|(id, _)| id.as_str()).collect();
        return Err(format!(
            "there is no mind `{mind}` on this desktop; `describe shell` lists them under `workroom.minds`: {}.",
            if known.is_empty() { "none".to_string() } else { known.join(", ") }
        ));
    }
    Ok(Asked { page, mind: Some(mind.to_string()) })
}

/// What the screen says after the call, and a `note` for anything that is not as asked: a page or
/// narrowing the screen did not take, a screen that is not showing, a shell still behind another
/// window. The caller reads the answer, not a guess that it worked.
fn read_back(asked: &Asked, page: &str, narrowed: &str, on_screen: bool, raised: bool) -> Value {
    let narrowed = Some(narrowed.to_string()).filter(|m| !m.is_empty());
    let mut notes: Vec<String> = Vec::new();
    // A mind's narrowing is the workroom page whatever page was asked.
    if asked.mind.is_none() && page != asked.page {
        notes.push(format!("the screen is on `{page}`, not `{}`", asked.page));
    }
    if asked.mind != narrowed && !(asked.mind.is_none() && asked.page != "workroom") {
        notes.push(format!(
            "asked to narrow to {}, but the screen is narrowed to {}",
            asked.mind.as_deref().unwrap_or("no mind"),
            narrowed.as_deref().unwrap_or("no mind")
        ));
    }
    if !on_screen {
        notes.push("the Agents screen is not the one showing".to_string());
    }
    if !raised {
        notes.push("the shell could not be brought in front of the window covering it".to_string());
    }
    let mut out = json!({ "page": page, "narrowed_to": narrowed, "on_screen": on_screen, "raised": raised });
    if !notes.is_empty() {
        out["note"] = notes.join("; ").into();
    }
    out
}

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let ui = ui.as_weak();
    surface.action(spec(), move |args| {
        let asked = asked(args, &crate::wire::agents::workroom_minds_now())?;
        // Raising the shell moves the person's view: held while a card waits, like every act that does.
        crate::card_watch::hold_windows("show_workroom")?;
        let ui = ui.upgrade().ok_or("the shell is gone")?;
        let g = ui.global::<AgentsState>();
        // The screen's own handlers, the ones its navigation calls: one path, whoever pressed.
        match &asked.mind {
            Some(mind) => g.invoke_filter_mind(mind.as_str().into()),
            None => g.invoke_show_section(asked.page.into()),
        }
        ui.set_current_screen(crate::wire::agents::SCREEN);
        ui.invoke_navigate(crate::wire::agents::SCREEN);
        let raised = crate::windows::raise_shell().is_ok();
        // The observed state, read back off the screen, not an `accepted: true`.
        Ok(read_back(
            &asked,
            g.get_section().as_str(),
            &g.get_mind_filter(),
            ui.get_current_screen() == crate::wire::agents::SCREEN,
            raised,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minds() -> Vec<(String, String)> {
        vec![("pi".into(), "pi".into()), ("hermes".into(), "Hermes".into())]
    }

    /// Showing a page changes what is drawn and nothing about any agent: `safe`, as `show_agent` is.
    #[test]
    fn show_workroom_is_safe_and_takes_no_token_or_agent() {
        let spec = spec();
        assert_eq!(spec.permission, "safe");
        let schema = spec.schema();
        let params = schema["parameters"]["properties"].as_object().unwrap();
        for banned in ["agent", "agent_token", "token", "caller", "grant"] {
            assert!(!params.contains_key(banned), "show_workroom takes `{banned}`: {schema}");
        }
        assert!(params.contains_key("page") && params.contains_key("mind"));
    }

    #[test]
    fn a_page_is_one_of_the_navigations_and_the_default_is_the_workroom() {
        assert_eq!(asked(&json!({}), &minds()).unwrap(), Asked { page: "workroom", mind: None });
        assert_eq!(asked(&json!({ "page": " history " }), &minds()).unwrap().page, "history");
        assert_eq!(asked(&json!({ "page": "needs_you" }), &minds()).unwrap().page, "needs_you");
        let err = asked(&json!({ "page": "tasks" }), &minds()).unwrap_err();
        assert!(err.contains("workroom, needs_you, history"), "{err}");
    }

    #[test]
    fn a_mind_narrows_the_workroom_only_and_must_exist() {
        let one = asked(&json!({ "mind": "hermes" }), &minds()).unwrap();
        assert_eq!(one, Asked { page: "workroom", mind: Some("hermes".into()) });
        let err = asked(&json!({ "page": "history", "mind": "pi" }), &minds()).unwrap_err();
        assert!(err.contains("workroom page only"), "{err}");
        let err = asked(&json!({ "mind": "ghost" }), &minds()).unwrap_err();
        assert!(err.contains("no mind `ghost`") && err.contains("pi, hermes"), "{err}");
    }

    /// A card waiting holds it before anything moves, and the raise is after the hold.
    #[test]
    fn show_workroom_asks_hold_windows_before_it_moves_the_view() {
        let src = include_str!("control_workroom.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        let held = src.find("card_watch::hold_windows(\"show_workroom\")").expect("show_workroom asks hold_windows");
        let moved = src.find("invoke_show_section").expect("it changes the page");
        let raised = src.find("raise_shell()").expect("it brings the shell forward");
        assert!(held < moved && held < raised, "the hold comes before anything moves");
    }

    /// Every pointer path on the workroom has its word on the surface (the table at the top).
    #[test]
    fn every_workroom_act_has_its_action_on_the_surface() {
        let agents = include_str!("control_agents.rs");
        for action in ["\"new_agent\"", "\"hand_off\"", "\"show_agent\"", "\"stop_agent\""] {
            assert!(agents.contains(action), "{action} is not published");
        }
        let control = include_str!("control.rs");
        let control = control.split("#[cfg(test)]").next().unwrap();
        for action in ["\"use_harness\"", "\"open_lens\""] {
            assert!(control.contains(action), "{action} is not published");
        }
        assert!(control.contains("crate::control_workroom::actions("), "show_workroom is registered");
        assert!(control.contains("crate::wire::agents::workroom_for_describe("), "describe shell reads the workroom");
    }

    /// Review of #584, finding 6: a narrowing the screen ignored, or a raise that failed, is in the
    /// answer and not only in the state it reports.
    #[test]
    fn the_answer_says_when_the_screen_is_not_as_asked() {
        let asked_hermes = Asked { page: "workroom", mind: Some("hermes".into()) };
        let ok = read_back(&asked_hermes, "workroom", "hermes", true, true);
        assert!(ok.get("note").is_none(), "{ok}");
        assert_eq!(ok["narrowed_to"], "hermes");
        let ignored = read_back(&asked_hermes, "workroom", "", true, true);
        assert!(ignored["note"].as_str().unwrap().contains("narrowed to no mind"), "{ignored}");
        let history = Asked { page: "history", mind: None };
        assert!(read_back(&history, "history", "", true, true).get("note").is_none());
        assert!(read_back(&history, "workroom", "", true, true)["note"].as_str().unwrap().contains("not `history`"));
        let behind = read_back(&history, "history", "", true, false);
        assert_eq!(behind["raised"], false);
        assert!(behind["note"].as_str().unwrap().contains("brought in front"), "{behind}");
        assert!(read_back(&history, "history", "", false, true)["note"].as_str().unwrap().contains("not the one showing"));
    }
}
