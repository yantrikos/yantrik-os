//! `record_gate_decision`: an app's gate has let a call run unasked, and the shell writes the line
//! (#1353, round 4 of #1357).
//!
//! Every unasked action is written down by the shell, never by its caller. For a call that met an
//! app's socket directly, the shell's request path was never in it, so the app's own gate
//! (`yantrik_ipc_transport::unasked`) asks the shell to write the line before it runs the call, and
//! refuses the call if the shell does not say it wrote it. The shell's own registry writes the
//! same line in-process.
//!
//! What a caller can and cannot do here: it can make the shell write a line saying "this call
//! ran unasked, as reported by this program", labelled with who the kernel says is asking, which
//! is a true statement about what it asked. The mode on the line is the shell's own, never the
//! caller's. The Mind cannot send it at all (`app_guard::REFUSED`), and nothing here runs, allows
//! or changes anything.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};
use yantrik_ipc_transport::unasked;

pub fn actions(surface: ControlSurface) -> ControlSurface {
    // The shell's own registry is a gate too: it writes the line without a socket.
    unasked::write_with(|r| write(&format!("the shell's own gate for {}", r.app), r.app, r.action, r.grade, r.mode, r.args, true));
    surface.action(
        Action::new(
            unasked::ACTION,
            "Called by an app's own gate, before it runs a call without asking in a bypass mode: the \
             shell writes the line in the record of what ran unasked, with the whole arguments. It \
             allows nothing, runs nothing and changes no setting.",
        )
        .risk("safe")
        .arg(Param::text("app").describe("The app whose gate let the call through"))
        .arg(Param::text("action").describe("The action"))
        .arg(Param::text("grade").describe("The grade it is published at"))
        .arg(Param::text("mode").optional().describe("The mode the gate read; the shell writes its own"))
        .arg(Param::text("args_json").describe("The whole arguments, as JSON text")),
        |args| {
            let field = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let parsed: serde_json::Value = serde_json::from_str(&field("args_json")).map_err(|e| format!("`args_json` is not JSON ({e})"))?;
            let asker = crate::control_approvals::who_is_asking("an app's gate");
            let by = format!("an app's gate ({})", asker.line);
            let (app, action) = (field("app"), field("action"));
            if app.is_empty() || action.is_empty() {
                return Err("`app` and `action` are required".into());
            }
            write(&by, &app, &action, &field("grade"), &field("mode"), &parsed, false)?;
            Ok(serde_json::json!({ "written": true }))
        },
    )
}

/// The line, written by the shell with the mode it is in now. `Err` when it could not be written
/// whole.
fn write(by: &str, app: &str, action: &str, grade: &str, claimed: &str, args: &serde_json::Value, own: bool) -> Result<(), String> {
    let mode = crate::mind_mode::current().as_str().to_string();
    // The mode on the line is the shell's own: a gate that read another one (the mode changed
    // between, or it is mistaken) is noted beside it, never believed.
    let on_line = if own || claimed.is_empty() || claimed == mode { mode } else { format!("{mode} (the gate read {claimed})") };
    let verified = if own { crate::approvals::Verified::default() } else { crate::control_approvals::who_is_asking(by) };
    crate::unasked_audit::at_decision(&on_line, by, &verified, app, action, args, grade)
}

#[cfg(test)]
#[path = "control_gate_audit_tests.rs"]
mod tests;
