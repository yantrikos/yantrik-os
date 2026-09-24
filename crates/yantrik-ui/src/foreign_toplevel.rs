//! Un-minimising a window without maximising it, over the wlr foreign-toplevel protocol (#265).
//!
//! The only verb wlrctl 0.2.2 has that brings a minimised window back is `maximize`, so a window
//! restored from the taskbar came back maximised whatever size it went away at. The protocol
//! underneath has a request for exactly this — `zwlr_foreign_toplevel_handle_v1.unset_minimized` —
//! and labwc implements it; wlrctl simply does not expose it. So the shell speaks the protocol
//! itself: a second Wayland connection, open only for the length of one restore, asking for
//! nothing but one window's un-minimise and activate.
//!
//! The part that can be wrong without a compositor in the room — which toplevel is the one named —
//! is the pure function [`pick`], with its test table at the bottom of this file. The part that
//! needs a compositor reports what it found ([`Restore`]) so `windows::present` can fall back to
//! the old `wlrctl maximize` and say in the log that it did, when this protocol is not offered.

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as handle_proto,
    zwlr_foreign_toplevel_manager_v1 as manager_proto,
};

/// One toplevel the compositor announced, as plain data: what the protocol says about a window,
/// kept apart from the proxy that says it so choosing between windows is testable without a
/// compositor.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToplevelInfo {
    pub title: String,
    pub app_id: String,
    pub minimized: bool,
}

/// What one restore attempt found out.
pub(crate) enum Restore {
    /// The named window was found; if it was minimised it has been asked to come back at its old
    /// size, and it has been asked to take focus.
    Restored,
    /// The protocol worked and no window answers to that name — the same fact wlrctl reported as
    /// a non-zero exit, meaning the taskbar and the compositor disagree (or the window was never
    /// minimised under a name this caller had).
    NothingNamed,
    /// The protocol could not be used at all — no compositor socket, the global is not offered —
    /// with the reason in words. `windows::present` falls back to `wlrctl maximize` on this, and
    /// logs that it did.
    Unavailable(String),
}

/// Restore the window called `title` through the protocol: `unset_minimized` if it is minimised,
/// then `activate`.
///
/// `app_id`, when given, is the fallback name `windows::matchspecs` decided to offer for the same
/// window, spelled as the compositor spelled it — the protocol names windows by exactly the two
/// keys wlrctl does, under the same rules; see [`pick`].
///
/// This runs on the caller's thread, which is the UI thread, like the `wlrctl` calls it replaces
/// in `present`: a handful of roundtrips the compositor answers at once, milliseconds in practice.
pub(crate) fn restore(title: &str, app_id: Option<&str>) -> Restore {
    match talk(title, app_id) {
        Ok(outcome) => outcome,
        Err(why) => Restore::Unavailable(why),
    }
}

/// The whole conversation with the compositor. `Err` is only for "the protocol could not be
/// used", the [`Restore::Unavailable`] case; a protocol that worked and matched nothing is
/// [`Restore::NothingNamed`].
fn talk(want_title: &str, want_app_id: Option<&str>) -> Result<Restore, String> {
    let conn = Connection::connect_to_env()
        .map_err(|e| format!("this session has no Wayland compositor socket to open: {e}"))?;
    let (globals, mut queue) = registry_queue_init(&conn)
        .map_err(|e| format!("the compositor did not hand over its list of globals: {e}"))?;
    let qh = queue.handle();

    // Version 1 of the manager already carries unset_minimized; binding up to the newest the
    // protocol knows lets a compositor that implements more still be talked to.
    let _manager: manager_proto::ZwlrForeignToplevelManagerV1 = globals
        .bind(&qh, 1..=3, ())
        .map_err(|_| {
            "the compositor does not offer zwlr_foreign_toplevel_manager_v1, which is where \
             unset_minimized lives"
                .to_string()
        })?;
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=9, ())
        .map_err(|_| "the compositor offers no wl_seat, and activate has to name one".to_string())?;

    let mut talker = Talker { toplevels: Vec::new(), finished: false };

    // The manager announces every toplevel it has and then `finished`; each handle ends its
    // first burst of news with `done`. The bound is for a compositor that announces windows
    // but never says `done`: deciding on the news that did arrive beats freezing the desktop
    // waiting for the rest.
    const MOST_ROUNDS: usize = 16;
    for _ in 0..MOST_ROUNDS {
        if talker.finished && talker.toplevels.iter().all(|t| t.ready) {
            break;
        }
        queue
            .roundtrip(&mut talker)
            .map_err(|e| format!("the conversation about open windows broke off: {e}"))?;
    }
    if !talker.finished {
        return Err("the compositor never finished announcing its windows, so the list cannot \
                    be trusted"
            .to_string());
    }

    let open: Vec<&Seen> = talker.toplevels.iter().filter(|t| !t.closed).collect();
    let infos: Vec<ToplevelInfo> = open.iter().map(|t| t.info.clone()).collect();
    let Some(idx) = pick(want_title, want_app_id, &infos) else {
        return Ok(Restore::NothingNamed);
    };
    let window = open[idx];
    if window.info.minimized {
        window.handle.unset_minimized();
    }
    window.handle.activate(&seat);
    // A roundtrip rather than a flush: it says the requests reached the compositor before this
    // connection is dropped and the caller moves on to wlrctl's `focus`.
    queue
        .roundtrip(&mut talker)
        .map_err(|e| format!("the compositor did not take the restore request: {e}"))?;
    Ok(Restore::Restored)
}

/// The toplevel a restore should act on, or `None` when none of them can be meant.
///
/// The title is matched EXACTLY, as wlrctl's `title:` does: callers resolve what a person or a
/// mind said against the open window list before they get here (`windows::window_named`), and a
/// loose match would restore the wrong window — "Notes" must not reach "Notes: Handover", which
/// is a different window with notes in it.
///
/// Where two toplevels share a title, the minimised one wins: the restore is about a window that
/// is away, and the one on screen is already where the click expects it.
///
/// The app_id fallback is the one `windows::matchspecs` decided to offer — spelled as the
/// compositor spelled it, and only for a window alone under that app_id — and it exists for the
/// same reason it does there: a foreign window's title moves (Blender retitles on save, Chromium
/// on every tab) and the list the caller resolved against is up to nine seconds old, so the exact
/// title can miss a window that is plainly there under its app_id.
pub(crate) fn pick(want_title: &str, want_app_id: Option<&str>, open: &[ToplevelInfo]) -> Option<usize> {
    fn minimised_first(matches: Vec<usize>, open: &[ToplevelInfo]) -> Option<usize> {
        matches
            .iter()
            .copied()
            .find(|i| open[*i].minimized)
            .or_else(|| matches.first().copied())
    }
    let by_title: Vec<usize> = open
        .iter()
        .enumerate()
        .filter(|(_, w)| w.title == want_title)
        .map(|(i, _)| i)
        .collect();
    if !by_title.is_empty() {
        return minimised_first(by_title, open);
    }
    if let Some(id) = want_app_id {
        let by_id: Vec<usize> = open
            .iter()
            .enumerate()
            .filter(|(_, w)| w.app_id == id)
            .map(|(i, _)| i)
            .collect();
        if !by_id.is_empty() {
            return minimised_first(by_id, open);
        }
    }
    None
}

// ── The Wayland plumbing ─────────────────────────────────────────────

/// One announced toplevel: the proxy that can be asked to move, and what has been said about it.
struct Seen {
    handle: handle_proto::ZwlrForeignToplevelHandleV1,
    info: ToplevelInfo,
    /// The handle's first `done`: the compositor has said all it meant to say in this burst.
    ready: bool,
    closed: bool,
}

/// The state of one short conversation. Everything it collects lands in `toplevels`; the
/// `Dispatch` impls below only ever fill fields of it.
struct Talker {
    toplevels: Vec<Seen>,
    /// The manager's `finished`: every toplevel that existed at bind time has been announced.
    finished: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Talker {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The globals list this needs was read by `registry_queue_init` before any event can
        // reach this queue; a global appearing mid-conversation is not one this restore is for.
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Talker {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The seat is only ever named in an `activate` request; nothing it reports matters here.
    }
}

impl Dispatch<manager_proto::ZwlrForeignToplevelManagerV1, ()> for Talker {
    fn event(
        state: &mut Self,
        _: &manager_proto::ZwlrForeignToplevelManagerV1,
        event: manager_proto::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager_proto::Event::Toplevel { toplevel } => state.toplevels.push(Seen {
                handle: toplevel,
                info: ToplevelInfo { title: String::new(), app_id: String::new(), minimized: false },
                ready: false,
                closed: false,
            }),
            manager_proto::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    // The `toplevel` event carries a proxy the compositor created, and the generated dispatch
    // needs to be told what user data to hang on it before the handle's own events can arrive.
    wayland_client::event_created_child!(Talker, manager_proto::ZwlrForeignToplevelManagerV1, [
        manager_proto::EVT_TOPLEVEL_OPCODE => (handle_proto::ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<handle_proto::ZwlrForeignToplevelHandleV1, ()> for Talker {
    fn event(
        state: &mut Self,
        handle: &handle_proto::ZwlrForeignToplevelHandleV1,
        event: handle_proto::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(seen) = state.toplevels.iter_mut().find(|s| s.handle.id() == handle.id()) else {
            return;
        };
        match event {
            handle_proto::Event::Title { title } => seen.info.title = title,
            handle_proto::Event::AppId { app_id } => seen.info.app_id = app_id,
            // The state array is raw bytes, one native-endian u32 per flag.
            handle_proto::Event::State { state } => {
                seen.info.minimized = state
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
                    .any(|flag| flag == handle_proto::State::Minimized as u32);
            }
            handle_proto::Event::Done => seen.ready = true,
            handle_proto::Event::Closed => seen.closed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str, app_id: &str, minimized: bool) -> ToplevelInfo {
        ToplevelInfo { title: title.into(), app_id: app_id.into(), minimized }
    }

    /// The choice `talk` makes with what the compositor announced — every rule in `pick`'s doc,
    /// one row each, because a wrong pick here un-minimises somebody else's work.
    #[test]
    fn the_named_window_is_the_one_restored() {
        let open = vec![
            window("Terminal", "", false),
            window("Notes", "", true),
            window("Notes: Handover", "", false),
        ];
        // The exact title, and only the exact title: "Notes" must not reach "Notes: Handover",
        // which is a different window with notes in it.
        assert_eq!(pick("Notes", None, &open), Some(1));
        assert_eq!(pick("Notes: Handover", None, &open), Some(2));
        // Nothing open answers to the name — the fact wlrctl reported as a non-zero exit.
        assert_eq!(pick("Editor", None, &open), None);
        // A loose match is not a match: `title:` was exact for wlrctl and stays exact here.
        assert_eq!(pick("note", None, &open), None);
        assert_eq!(pick("", None, &open), None);
    }

    /// Two toplevels sharing a title: the one away is the one a restore means, and the one on
    /// screen is already where the click expects it.
    #[test]
    fn a_minimised_window_wins_over_its_visible_twin() {
        let open = vec![window("Notes", "", false), window("Notes", "", true)];
        assert_eq!(pick("Notes", None, &open), Some(1));
        let flipped = vec![window("Notes", "", true), window("Notes", "", false)];
        assert_eq!(pick("Notes", None, &flipped), Some(0));
        // With none minimised the first is as good as any: the caller is about to focus it.
        let twins = vec![window("Notes", "", false), window("Notes", "", false)];
        assert_eq!(pick("Notes", None, &twins), Some(0));
    }

    /// A foreign window's title moves — Blender retitles on save, Chromium on every tab — and
    /// the list the caller resolved against is up to nine seconds old. The app_id `matchspecs`
    /// offered is the name that still finds it, spelled as the compositor spelled it:
    /// `app_id:blender` misses Blender's window exactly as it misses for wlrctl.
    #[test]
    fn a_title_that_has_moved_on_is_still_found_by_its_app_id() {
        let open = vec![
            window("Terminal", "", false),
            window("scene.blend - Blender 4.3.2", "Blender", true),
        ];
        assert_eq!(pick("(Unsaved) - Blender 4.3.2", Some("Blender"), &open), Some(1));
        assert_eq!(pick("(Unsaved) - Blender 4.3.2", Some("blender"), &open), None);
        assert_eq!(pick("(Unsaved) - Blender 4.3.2", None, &open), None);
        // The title still comes first when both would match different windows.
        let both = vec![
            window("Notes", "notes-app", false),
            window("Other", "Notes", true),
        ];
        assert_eq!(pick("Notes", Some("Notes"), &both), Some(0));
    }

    /// The restore the taskbar sends for a minimised window, end to end through the pure part:
    /// the window is found, it is the minimised one, and nothing about it says "maximize".
    #[test]
    fn the_choice_carries_the_minimised_state_the_caller_acts_on() {
        let open = vec![window("System Monitor", "", true)];
        let chosen = pick("System Monitor", None, &open).expect("the named window is there");
        assert!(open[chosen].minimized, "so `talk` sends unset_minimized, not wlrctl's maximize");
    }
}
