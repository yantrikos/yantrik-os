//! `caller_class`: whether a process is the person's or a mind's, as the shell reads it (#1085).
//!
//! The browser service has to send a mind's web tools to Mind View's browser and the person's own
//! to theirs. A mind such as Hermes runs as the person's account and carries no token, so the
//! account and the token cannot tell it from the person; what can is the shell's own record of
//! which minds are attached, and the walk up `/proc` from the caller's pid to one of them. That is
//! `requester_now` for a call made on the shell's socket. This answers it for a pid the browser
//! service saw on its own socket, with the same rule (`mind_view::classify`), so the choice of
//! browser and the choice of where a mind's app opens cannot disagree.
//!
//! Anything the shell cannot place is a mind: the answer `person` is only ever given to a process
//! that is not shown to descend from an attached mind.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::mind_view::{classify, CallerFacts, Requester};

/// `person` or `mind` for the process `pid`.
pub fn class_of(pid: i64) -> &'static str {
    let Ok(pid) = i32::try_from(pid) else { return "mind" };
    if pid <= 0 {
        return "mind";
    }
    let attached_mind = crate::caller_identity::resolve(pid).attached_mind;
    let facts = CallerFacts { pid: Some(pid as u32), agent: None, attached_mind, mind_account: false };
    match classify(&facts, std::process::id()) {
        Requester::Person => "person",
        Requester::Mind(_) => "mind",
    }
}

pub fn actions(surface: ControlSurface) -> ControlSurface {
    // The browser service starts Mind View's browser for a mind whose web call finds none: the
    // service is not the mind, so `open_app` would be routed as the person's.
    let surface = surface.action(
        Action::new(
            "open_mind_browser",
            "Start Mind View's own browser (never the person's Browser) for a mind's web tools, and              say why when it cannot. For the desktop's own services",
        )
        .risk("safe"),
        |_args| {
            let work = || crate::wire::browser_launch::open_in_mind_view().map(|()| serde_json::json!({ "started": true }));
            yantrik_app_runtime::control::answer_later(work)
                .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                .or_else(|work| work())
        },
    );
    surface.action(
        Action::new(
            "caller_class",
            "Whether a process is the person's own or a mind's, as the desktop reads it: `class` is \
             `person` or `mind`. For the desktop's own services, which choose a browser by it",
        )
        .risk("safe")
        .arg(Param::number("pid").describe("The process id as the kernel reported it on a socket")),
        |args| {
            let pid = args["pid"].as_i64().unwrap_or(0);
            // Off the UI thread: the browser service asks while it handles a call, and a /proc
            // walk is no business of the thread that draws the desktop.
            let work = move || Ok(serde_json::json!({ "class": class_of(pid) }));
            yantrik_app_runtime::control::answer_later(work)
                .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                .or_else(|work| work())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_cannot_be_placed_is_a_mind_never_the_person() {
        assert_eq!(class_of(0), "mind");
        assert_eq!(class_of(-4), "mind");
        assert_eq!(class_of(i64::MAX), "mind");
    }
}
