//! The one road to `app.act` for a model's call: `app_action`, and `await_app`'s `then_act`.
//!
//! Both name an app and an action, so both go through the same steps in the same order: what the call
//! names says whether it reaches out (refused before anything starts), the app is started, the app's own
//! declaration is read (refused when it fetches an address or runs a line), the person's cap is held,
//! and the act is made asking the person when the app says to. A second copy of these steps is a second
//! place to forget one; a source test (`tests/app_actions_reach.rs`) holds every companion tool that
//! sends `app.act` to this road or to fixed shell actions.

use serde_json::{json, Value};

use super::ToolContext;
use crate::app_reach;
use crate::app_ui::{act_asking, beyond_cap, describe, Desk};

/// `app_action`'s arguments for this app and action, which is how `reach` and `check_call` read it.
pub(crate) fn as_app_action(app: &str, action: &str, args: &Value) -> Value {
    json!({ "app": app, "action": action, "args": args })
}

/// What a call to this app's action is refused for before the app is started, or `None`.
pub(crate) fn refused_first(app: &str, action: &str, args: &Value) -> Option<String> {
    app_reach::refused_before_start(&as_app_action(app, action, args))
}

/// Make the act, or say why not. `Ok` is the app's reply; `Err` is a sentence for the model.
pub(crate) fn gated_act(ctx: &ToolContext, app: &str, action: &str, args: &Value, expect: &str) -> Result<Value, String> {
    if let Some(refusal) = refused_first(app, action, args) {
        return Err(refusal);
    }
    // A Yantrik app that is not running is started with no window first, and the act waits for it; one
    // that cannot run in time is dropped, never queued (#978 step 1).
    crate::app_start::ensure(app, true)?;
    let described = describe(app);
    if let Some(refusal) = app_reach::refused_unattended(app, action, described.as_ref().ok()) {
        return Err(refusal);
    }
    let mut purpose = String::new();
    if let Ok(ref view) = described {
        if let Some(refusal) = beyond_cap(view, app, action, ctx.max_permission) {
            return Err(refusal);
        }
        // The app's own sentence for the action, for the card if one has to go up.
        purpose = app_reach::purpose_of(view, action);
    }
    act_asking(&Desk::session(), app, action, args, expect, &purpose)
}
