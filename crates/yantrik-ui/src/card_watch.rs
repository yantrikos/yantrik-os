//! An approval card is never left behind another window.
//!
//! A card is drawn inside the shell's own window: the Mind panel, the Lens. When a card rises,
//! `control_approvals` raises the shell. But anything that takes focus afterwards covers it again:
//! a mind opening its next app into Mind View, a script, the person. On the live instance a
//! mind's `agent_run` card sat behind Mind View, maximised over the whole shell. The person at the
//! machine looked for it three times and could not see it, every card expired with the mind's
//! turn, and the conclusion was that approvals were broken (2 Oct 2026). A card is how a person
//! decides what a mind does. Hidden, it is a decision nobody can make.
//!
//! So while a card waits, a change of focus to any other window brings the shell back in front.
//! The changes come from `toplevel_watch`, which follows the compositor's own stream, so nothing
//! here polls. What two security reviews asked for shapes it:
//!
//! - **A mind cannot move windows while a card waits.** `focus_window`, `show_app`, `open_app`,
//!   `minimise_window`, `maximise_window`, `files_open`, `configure_harness`, the second press
//!   of `show_desktop`, `show_mind_audit` and the bar's three panel opens are refused on the control surface until the card
//!   is answered ([`hold_windows`]); bringing the shell itself forward is still allowed. Without
//!   that, a mind could spend the raises below by focusing an ordinary window six times, and the
//!   card would stay behind the sixth (second review, 2 Oct 2026). Any focus change still left is
//!   the person's own, or Mind View's, which the rules below answer. A call carrying a person's
//!   Allow is never held: that card has just been answered for exactly this act (final review).
//!   The surface cannot tell the person's own keybindings from a mind, so Ctrl+Alt+T and the
//!   second Super+D, which go through `yos`, wait for the card too.
//! - **It cannot be beaten by timing.** A window that takes focus back inside [`MIN_GAP`] is not
//!   ignored: a single recheck runs when the gap ends and raises if something other than the
//!   shell is still in front.
//! - **It cannot be worn out by a mind.** A person may switch away on purpose to check something
//!   before answering, and must not be fought for the keyboard, so other windows get at most
//!   [`MAX_RAISES`]. Mind View, the mind's own desktop, is never counted: flipping it in and out
//!   five times would otherwise buy a mind a permanently hidden card.
//! - **A raise cannot answer the card.** When the shell comes forward, the keyboard goes to a
//!   neutral scope, never the Lens's text field (the rest of whatever the person was typing
//!   would land there, and Enter would send it to the mind). For [`PRESS_GUARD`] after the shell
//!   comes forward, by whatever path, or after a different card is drawn where the person is
//!   looking ([`card_on_screen`]), a press on Allow is ignored: a click meant for the window that
//!   was just covered, or for the card that was just answered, can land on it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::windows::SHELL_WINDOW_TITLE;
use crate::App;

/// Two raises are at least this far apart, so that one focus change setting off another does not
/// become a loop between two windows. A change held back by it is rechecked when it ends.
const MIN_GAP: Duration = Duration::from_millis(1500);

/// How many times one stretch of waiting brings the shell back over windows that are not the
/// mind's own. After that the taskbar's Chat button, amber with a count, says a card is waiting.
const MAX_RAISES: u32 = 5;

/// After the shell comes forward, a press on Allow this soon is taken for a click meant for the
/// window that was just covered, and ignored.
pub const PRESS_GUARD: Duration = Duration::from_millis(700);

/// The mind's own desktop, which a mind controls. Its focus changes never use up the raises.
const MIND_VIEW_TITLE: &str = "Mind View";

static WAITING: AtomicBool = AtomicBool::new(false);
static RAISES: AtomicU32 = AtomicU32::new(0);
static RECHECK_PENDING: AtomicBool = AtomicBool::new(false);
static LAST: Mutex<Option<Instant>> = Mutex::new(None);
static RAISED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// The ids of the cards waiting now: a new id is a new card, whose count and guard start again.
static WAITING_IDS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// Which card is drawn where a person would press, and for which pane: see [`card_on_screen`].
static ON_SCREEN: Mutex<Option<Drawn>> = Mutex::new(None);
static SHELL_UI: OnceLock<slint::Weak<App>> = OnceLock::new();

/// Give this module the shell's window, so a raise can put the keyboard somewhere neutral.
pub fn attach(ui: slint::Weak<App>) {
    let _ = SHELL_UI.set(ui);
}

/// The approvals store's word on which cards are waiting, by id. A card that was not waiting
/// before starts the count and the gap again, so a harmless card left pending cannot have used
/// up the raises before the real one arrives (second review), and it starts the press guard: a
/// card painted under a pointer that was already pressing is not a decision.
pub fn set_waiting<'a>(ids: impl IntoIterator<Item = &'a str>) {
    let now: HashSet<String> = ids.into_iter().map(str::to_string).collect();
    WAITING.store(!now.is_empty(), Ordering::Relaxed);
    let new_card = match WAITING_IDS.lock() {
        Ok(mut seen) => {
            let fresh = now.iter().any(|id| !seen.as_ref().is_some_and(|s| s.contains(id)));
            *seen = Some(now);
            fresh
        }
        Err(_) => false,
    };
    if new_card {
        RAISES.store(0, Ordering::Relaxed);
        if let Ok(mut last) = LAST.lock() {
            *last = None;
        }
        shell_came_forward();
    }
}

/// Refuse an action that would move a window over the shell while a card waits. The shell's
/// handlers for those actions call this before they move anything.
///
/// Except a call that carries a person's Allow. The runtime spends the grant before the handler
/// runs, so with a second card waiting, refusing here used up the Allow on an act that then never
/// ran, and the person had said yes for nothing (final review of the card fix). They have just
/// answered a card for exactly this act: it runs, and if its window covers the card still
/// waiting, the raise below brings the shell back over it.
pub fn hold_windows(action: &str) -> Result<(), String> {
    held(WAITING.load(Ordering::Relaxed), yantrik_app_runtime::control::call_was_granted(), action)
}

/// Pure, for the tests. The refusal is read by a mind and, when a keybinding of the person's
/// went through `yos`, by nobody at all unless it says plainly what happened and what to do:
/// the surface cannot tell those two callers apart (final review).
fn held(waiting: bool, granted: bool, action: &str) -> Result<(), String> {
    if !waiting || granted {
        return Ok(());
    }
    Err(format!(
        "`{action}` was not run: an approval card is waiting for the person at this machine, and \
         no window is opened, moved or brought forward until it is answered, because a card \
         behind a window is a decision nobody can make. The card is on the desktop, and the \
         taskbar's Chat button counts it; answer it with Allow or Deny and `{action}` works \
         again. A mind reads it under `pending_approvals` in `describe shell` and asks again once \
         it is answered."
    ))
}

/// What a person pressing Allow would be pressing: the card drawn in front, the one drawn in the
/// agent pane on screen, and which pane that is.
#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    front: Option<String>,
    in_pane: Option<String>,
    pane: String,
}

/// The approvals store's word on which card is drawn where, every time it repaints. Another card
/// in the same place starts the press guard, as a card coming up does.
///
/// [`set_waiting`] starts it only for an id never seen before. When the front card is answered or
/// withdrawn, the next one, already waiting, is drawn in the same corner under the same pointer,
/// and a double click meant for the first card's Allow allowed the second, unread (final review
/// of the card fix). The same when the pane on screen changes and another card takes its place.
pub fn card_on_screen(front: Option<&str>, in_pane: Option<&str>, pane: &str) {
    let now = (front.is_some() || in_pane.is_some()).then(|| Drawn {
        front: front.map(str::to_string),
        in_pane: in_pane.map(str::to_string),
        pane: pane.to_string(),
    });
    let changed = match ON_SCREEN.lock() {
        Ok(mut shown) => {
            let changed = *shown != now;
            *shown = now.clone();
            changed
        }
        Err(_) => false,
    };
    // Nothing drawn is nothing to press by mistake.
    if changed && now.is_some() {
        start_press_guard();
    }
}

/// The shell has just come forward, by any path: card_watch's own raise, a card going up, the
/// Lens opening, a mind's `show_screen`. Each is a moment a click aimed at the window that was in
/// front can land on Allow, so each starts the press guard (second review: only card_watch's own
/// raises did, and a mind could choose the moment with any of the others).
pub fn shell_came_forward() {
    start_press_guard();
}

fn start_press_guard() {
    if let Ok(mut at) = RAISED_AT.lock() {
        *at = Some(Instant::now());
    }
}

/// Whether the shell came forward, or another card was drawn where a person presses, less than
/// [`PRESS_GUARD`] ago, so that a press on Allow now may be a click that was aimed somewhere else.
pub fn just_raised() -> bool {
    RAISED_AT
        .lock()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|at| at.elapsed() < PRESS_GUARD)
}

/// The compositor says another toplevel now has focus. Called from `toplevel_watch`'s thread.
pub fn front_changed(title: &str) {
    if !WAITING.load(Ordering::Relaxed) {
        return;
    }
    let since = LAST.lock().ok().and_then(|l| l.map(|at| at.elapsed()));
    match decide(true, title, crate::toplevel_watch::shell_titled_count(), since, RAISES.load(Ordering::Relaxed)) {
        Decision::Leave => {}
        Decision::Raise => raise(title),
        Decision::Later(wait) => recheck_after(wait),
    }
}

/// What to do about a change of focus.
#[derive(Debug, PartialEq)]
enum Decision {
    Leave,
    Raise,
    /// Inside the gap: look again when it ends.
    Later(Duration),
}

/// Pure, for the tests. `shells` is how many toplevels carry the shell's title: more than one
/// means a window is wearing it, and a title alone can no longer say the shell is in front.
fn decide(waiting: bool, title: &str, shells: usize, since_last: Option<Duration>, raises: u32) -> Decision {
    if !waiting {
        return Decision::Leave;
    }
    // The shell in front is the card in front, unless something else has taken its name. An
    // untitled window is not the shell: ours always says what it is.
    if title == SHELL_WINDOW_TITLE && shells <= 1 {
        return Decision::Leave;
    }
    if title != MIND_VIEW_TITLE && raises >= MAX_RAISES {
        return Decision::Leave;
    }
    match since_last {
        Some(gap) if gap < MIN_GAP => Decision::Later(MIN_GAP - gap),
        _ => Decision::Raise,
    }
}

fn raise(title: &str) {
    // Claimed, not counted after the fact: two focus changes deciding at once must not both get
    // the fifth raise (second review).
    if title != MIND_VIEW_TITLE && !claim_a_raise() {
        return;
    }
    if let Ok(mut last) = LAST.lock() {
        *last = Some(Instant::now());
    }
    if title == SHELL_WINDOW_TITLE {
        tracing::warn!("a second window is titled like the shell while a card waits; bringing the shell forward anyway");
    }
    // Off this thread: the raise waits on the compositor, and the stream must keep flowing.
    std::thread::spawn(|| {
        // The card may have been answered while this was being decided. Raising now would take
        // the screen back from the window the hand-back just returned it to.
        if !WAITING.load(Ordering::Relaxed) {
            return;
        }
        match crate::windows::raise_shell() {
            Ok(()) => {
                shell_came_forward();
                if let Some(ui) = SHELL_UI.get() {
                    let _ = ui.upgrade_in_event_loop(|ui| ui.invoke_focus_global_keys());
                }
                tracing::info!("brought the shell back in front of a window that covered a waiting card");
            }
            Err(why) => tracing::warn!(%why, "a card is waiting behind another window and the shell could not come back in front"),
        }
    });
}

/// Take one of the raises other windows are allowed, if any is left.
fn claim_a_raise() -> bool {
    RAISES
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |r| (r < MAX_RAISES).then_some(r + 1))
        .is_ok()
}

/// One recheck at a time: when the gap ends, raise if a card still waits and something other
/// than the shell is in front.
fn recheck_after(wait: Duration) {
    if RECHECK_PENDING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        RECHECK_PENDING.store(false, Ordering::Relaxed);
        if let Some(front) = crate::toplevel_watch::front_title() {
            front_changed(&front);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case from the live instance: a card waiting, and Mind View takes focus over it.
    #[test]
    fn a_window_taking_focus_over_a_waiting_card_brings_the_shell_back() {
        assert_eq!(decide(true, "Mind View", 1, None, 0), Decision::Raise);
        assert_eq!(decide(true, "Notes", 1, Some(Duration::from_secs(3)), 2), Decision::Raise);
    }

    #[test]
    fn nothing_waiting_takes_nothing_from_anyone() {
        assert_eq!(decide(false, "Mind View", 1, None, 0), Decision::Leave);
    }

    /// The shell taking focus is the card coming forward, not something covering it.
    #[test]
    fn the_shell_itself_is_not_covering_the_card() {
        assert_eq!(decide(true, SHELL_WINDOW_TITLE, 1, None, 0), Decision::Leave);
    }

    /// Review finding: a window titled like the shell, or with no title, must not pass for it.
    #[test]
    fn a_window_wearing_the_shells_name_or_none_is_covering() {
        assert_eq!(decide(true, SHELL_WINDOW_TITLE, 2, None, 0), Decision::Raise);
        assert_eq!(decide(true, "", 1, None, 0), Decision::Raise);
    }

    /// Review finding: a re-cover inside the gap used to be ignored for good. It is now looked
    /// at again when the gap ends.
    #[test]
    fn a_window_that_covers_again_inside_the_gap_is_rechecked_not_forgotten() {
        assert_eq!(
            decide(true, "Mind View", 1, Some(Duration::from_millis(200)), 1),
            Decision::Later(Duration::from_millis(1300))
        );
    }

    /// A person who keeps switching to another app is doing it on purpose and is left alone; a
    /// mind flipping its own desktop in and out cannot use the raises up.
    #[test]
    fn the_cap_leaves_a_person_alone_but_never_lets_mind_view_win() {
        assert_eq!(decide(true, "Notes", 1, None, MAX_RAISES), Decision::Leave);
        assert_eq!(decide(true, "Mind View", 1, None, MAX_RAISES + 20), Decision::Raise);
    }

    /// The statics are shared, so the tests that touch them take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// Second review: a harmless card kept pending could use up the raises before the real one
    /// arrived. Each new card starts the count and the gap again, and only a new one does.
    #[test]
    fn a_new_card_counts_and_times_again_and_an_old_one_does_not() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_waiting([]);
        set_waiting(["decoy"]);
        RAISES.store(MAX_RAISES, Ordering::Relaxed);
        *LAST.lock().unwrap() = Some(Instant::now());
        set_waiting(["decoy"]);
        assert_eq!(RAISES.load(Ordering::Relaxed), MAX_RAISES, "the same card waiting on is not a new one");
        set_waiting(["decoy", "real"]);
        assert_eq!(RAISES.load(Ordering::Relaxed), 0, "a new card starts the count again");
        assert!(LAST.lock().unwrap().is_none());
        assert!(just_raised(), "and a card just painted is not answered by a press already on its way");
        set_waiting([]);
        *RAISED_AT.lock().unwrap() = None;
    }

    /// Second review: a mind focusing an ordinary window six times spent the raises and kept the
    /// card behind the sixth. Window moves are refused while a card waits, and only then.
    #[test]
    fn window_moves_are_held_while_a_card_waits_and_only_then() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_waiting([]);
        assert!(hold_windows("focus_window").is_ok());
        set_waiting(["c1"]);
        let why = hold_windows("focus_window").unwrap_err();
        assert!(why.contains("`focus_window` was not run") && why.contains("pending_approvals"), "{why}");
        set_waiting([]);
        assert!(hold_windows("focus_window").is_ok(), "answered, and windows move again");
        *RAISED_AT.lock().unwrap() = None;
    }

    /// Final review: with two cards waiting, the act a person allowed on one was refused for the
    /// other, after the runtime had spent the Allow. A call that carries a grant is never held.
    #[test]
    fn an_act_the_person_allowed_is_not_held_for_another_card() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_waiting(["the-other-card"]);
        assert!(hold_windows("open_app").is_err(), "unallowed, it waits for the card");
        {
            let _allowed = yantrik_app_runtime::control::GrantedScope::enter(true);
            assert!(hold_windows("open_app").is_ok(), "the Allow was spent for this act, so it runs");
        }
        assert!(hold_windows("open_app").is_err(), "and the next call, unallowed, waits again");
        set_waiting([]);
        *RAISED_AT.lock().unwrap() = None;
    }

    /// Final review: Ctrl+Alt+T goes through `yos open_app` and is held like a mind's call. The
    /// refusal is the only thing anyone is told, so it has to say what to do, to a person too.
    #[test]
    fn a_held_window_says_where_the_card_is_and_how_to_go_on() {
        let why = held(true, false, "open_app").unwrap_err();
        for words in ["`open_app` was not run", "approval card is waiting", "Chat button", "Allow or Deny"] {
            assert!(why.contains(words), "missing {words:?}: {why}");
        }
        assert!(held(false, false, "open_app").is_ok());
        assert!(held(true, true, "open_app").is_ok());
    }

    /// Final review: answering the card in front drew the next one in the same corner, under the
    /// same pointer, with no guard, so a double click allowed a card nobody had read. Another card
    /// where the person presses starts the guard; the same card repainted does not.
    #[test]
    fn the_next_card_drawn_in_the_same_place_is_not_answered_by_the_last_click() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        card_on_screen(None, None, "");
        *RAISED_AT.lock().unwrap() = None;

        card_on_screen(Some("first"), None, "");
        assert!(just_raised(), "a card drawn is not answered by a press already on its way");
        *RAISED_AT.lock().unwrap() = None;
        card_on_screen(Some("first"), None, "");
        assert!(!just_raised(), "the same card repainted is not another card");

        card_on_screen(Some("second"), None, "");
        assert!(just_raised(), "the first answered, the second in its place");
        *RAISED_AT.lock().unwrap() = None;

        card_on_screen(Some("second"), Some("pane-card"), "agent-7");
        assert!(just_raised(), "a card drawn in the pane on screen");
        *RAISED_AT.lock().unwrap() = None;
        card_on_screen(Some("second"), Some("pane-card"), "agent-8");
        assert!(just_raised(), "another pane, another place to press");
        *RAISED_AT.lock().unwrap() = None;

        card_on_screen(None, None, "agent-8");
        assert!(!just_raised(), "nothing drawn is nothing to press");
        *RAISED_AT.lock().unwrap() = None;
    }

    /// The fifth raise is claimed once, however many focus changes decide at the same moment.
    #[test]
    fn the_cap_is_claimed_not_counted_after() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        RAISES.store(MAX_RAISES - 1, Ordering::Relaxed);
        let won = (0..8).filter(|_| claim_a_raise()).count();
        assert_eq!(won, 1);
        RAISES.store(0, Ordering::Relaxed);
    }

    /// The handlers that move windows are the ones that ask first. Read from the source, the way
    /// `published_actions_cannot_grant` reads it, so that a new window verb added without the
    /// hold fails here rather than in front of a person. The final review found what the second
    /// one missed: a file opened into its viewer, a harness's setup terminal, Show desktop's
    /// second press bringing every window back, the mode menu drawn over the card, and the bar's
    /// panels drawn over the Lens and the card in its chat.
    #[test]
    fn every_window_moving_action_asks_hold_windows_first() {
        let held: [(&str, &[&str]); 4] = [
            (
                "control.rs",
                &["focus_window", "show_app", "open_app", "minimise_window", "maximise_window", "configure_harness", "show_desktop"],
            ),
            ("control_files.rs", &["files_open"]),
            ("control_approvals.rs", &["show_mind_audit"]),
            ("control_overlays.rs", &["open_quick_settings", "open_power_menu", "open_clipboard", "open_cheat_sheet"]),
        ];
        for (file, actions) in held {
            let src = std::fs::read_to_string(format!("{}/src/{file}", env!("CARGO_MANIFEST_DIR"))).unwrap();
            for action in actions {
                assert!(
                    src.contains(&format!("crate::card_watch::hold_windows(\"{action}\")")),
                    "`{action}` in {file} moves a window and does not call hold_windows; a card could be covered"
                );
            }
        }
    }

    #[test]
    fn a_press_right_after_a_raise_is_not_taken_for_an_answer() {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *RAISED_AT.lock().unwrap() = Some(Instant::now());
        assert!(just_raised());
        *RAISED_AT.lock().unwrap() = Some(Instant::now() - PRESS_GUARD - Duration::from_millis(10));
        assert!(!just_raised());
        *RAISED_AT.lock().unwrap() = None;
    }
}
