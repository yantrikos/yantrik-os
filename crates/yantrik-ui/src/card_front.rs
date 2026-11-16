//! Getting in front of the person for an approval card, and getting out of the way again.
//!
//! A card the person cannot see is the same as no card: the request expires on its own and they
//! are never told anything was asked. So the shell comes forward when a request arrives. The cost
//! is that it covers whatever they were using, which is why the window they were in is handed the
//! screen back the moment nothing is waiting.
//!
//! Order of events for a read of the desktop (`read_screen`, `screenshot_screen`) that asks:
//!
//! - Before round 14: the card went up and the shell came forward; the person pressed Allow; the
//!   companion's next call carried the grant and the read ran at once. The hand-back is done on
//!   the next repaint of the cards, on a thread of its own, so it had not happened when the read
//!   took its picture: the picture was of the shell's own home screen.
//! - Now: the read calls [`settle_before_read`] first, which gives the screen back itself, waits
//!   for a hand-back already under way, and lets the compositor draw the window before the
//!   picture is taken.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The toplevel to hand the screen back to, if it was knowable when the card went up.
///
/// `None` means either nothing is waiting, or the compositor would not say unambiguously which
/// window was in front — in which case the shell stays where it is rather than guessing at a
/// window to throw the person into. See [`crate::windows::front_now`].
static RESTORE_TO: Mutex<Option<String>> = Mutex::new(None);

/// Hand-backs on their way: a thread has taken the title and the compositor has not been told yet.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// How long a read waits for a hand-back that is on its way.
const MOST_WAIT: Duration = Duration::from_secs(2);

/// After the compositor is told to focus the window, how long it is given to draw it.
const DRAW_TIME: Duration = Duration::from_millis(400);

/// Ask the compositor to bring one toplevel forward. Blocking; call it off the UI thread.
fn focus_toplevel(title: &str) {
    match std::process::Command::new("wlrctl")
        .args(["toplevel", "focus", &format!("title:{title}")])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(
            code = status.code().unwrap_or(-1),
            window = %title,
            "could not bring a window forward for an approval"
        ),
        Err(e) => tracing::warn!(error = %e, "could not run wlrctl for an approval"),
    }
}

/// Note what the person was using, then put the shell in front of it.
///
/// Both halves on one worker thread and in that order, because the reading has to happen before
/// the raise or it reads the shell. It asks the compositor fresh rather than reading the taskbar's
/// cached answer, because that one is up to `COMPOSITOR_TTL` old and the screen is being handed to
/// the window in front NOW. Nothing is recorded if something is already waiting — the
/// shell is already in front by then, so a second reading would capture the shell and the window
/// the person actually came from would be lost.
pub fn take_the_screen() {
    std::thread::spawn(|| {
        if let Ok(mut slot) = RESTORE_TO.lock() {
            if slot.is_none() {
                *slot = crate::windows::front_now();
            }
        }
        focus_toplevel(crate::windows::SHELL_WINDOW_TITLE);
    });
}

/// Nothing is waiting any more: give the screen back to whatever the person was using.
///
/// Called on every transition to "no pending requests", so it covers a decision and an expiry
/// alike — the person who walked away and came back should find the window they left, not the
/// shell they never answered.
pub fn give_the_screen_back() {
    if let Some(title) = take_title(&RESTORE_TO, &IN_FLIGHT) {
        std::thread::spawn(move || {
            focus_toplevel(&title);
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// Take the title out of `slot` and count a hand-back as on its way, under the slot's lock so a
/// reader that finds the slot empty also finds the count.
fn take_title(slot: &Mutex<Option<String>>, in_flight: &AtomicUsize) -> Option<String> {
    let mut slot = slot.lock().ok()?;
    let title = slot.take();
    if title.is_some() {
        in_flight.fetch_add(1, Ordering::SeqCst);
    }
    title
}

/// Before a read of the desktop that the person has just allowed: have the window they had in
/// front put back in front, and wait until it is drawn. Blocking; call it off the UI thread.
/// Does nothing, and does not wait, when no card took the screen (no card was asked, or the
/// compositor could not say which window was in front).
pub fn settle_before_read() {
    settle(&RESTORE_TO, &IN_FLIGHT, focus_toplevel, MOST_WAIT, DRAW_TIME);
}

fn settle(slot: &Mutex<Option<String>>, in_flight: &AtomicUsize, focus: impl Fn(&str), most: Duration, draw: Duration) {
    let mut acted = false;
    if let Some(title) = take_title(slot, in_flight) {
        focus(&title);
        in_flight.fetch_sub(1, Ordering::SeqCst);
        acted = true;
    }
    let until = Instant::now() + most;
    while in_flight.load(Ordering::SeqCst) > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
        acted = true;
    }
    if acted {
        std::thread::sleep(draw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_earlier_front_window_is_focused_before_the_read_and_only_once() {
        let slot = Mutex::new(Some("Notes".to_string()));
        let flight = AtomicUsize::new(0);
        let told = Mutex::new(Vec::new());
        settle(&slot, &flight, |t| told.lock().unwrap().push(t.to_string()), Duration::from_millis(50), Duration::ZERO);
        assert_eq!(*told.lock().unwrap(), ["Notes"]);
        settle(&slot, &flight, |t| told.lock().unwrap().push(t.to_string()), Duration::from_millis(50), Duration::ZERO);
        assert_eq!(told.lock().unwrap().len(), 1, "the window is handed the screen once");
    }

    #[test]
    fn a_read_waits_for_a_hand_back_already_on_its_way() {
        let slot = Mutex::new(Some("Notes".to_string()));
        let flight = Arc::new(AtomicUsize::new(0));
        assert_eq!(take_title(&slot, &flight).as_deref(), Some("Notes"));
        assert_eq!(flight.load(Ordering::SeqCst), 1);
        let done = flight.clone();
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            done.fetch_sub(1, Ordering::SeqCst);
        });
        let start = Instant::now();
        settle(&slot, &flight, |_| panic!("nothing left to focus"), Duration::from_secs(2), Duration::ZERO);
        assert!(start.elapsed() >= Duration::from_millis(100), "returned before the hand-back ended");
        late.join().unwrap();
    }

    #[test]
    fn with_no_card_a_read_neither_focuses_nor_waits() {
        let slot = Mutex::new(None);
        let flight = AtomicUsize::new(0);
        let start = Instant::now();
        settle(&slot, &flight, |_| panic!("no window to focus"), Duration::from_secs(2), Duration::from_secs(2));
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    /// The two reads of the desktop settle first, so nothing but `read` and the screenshot's worker
    /// can take a picture of the shell's own card screen.
    #[test]
    fn both_desktop_reads_settle_before_they_grab() {
        let screen = include_str!("control_screen.rs");
        assert!(screen.contains("card_front::settle_before_read()"));
        let capture = include_str!("control_capture.rs");
        assert!(capture.contains("card_front::settle_before_read()"));
    }
}
