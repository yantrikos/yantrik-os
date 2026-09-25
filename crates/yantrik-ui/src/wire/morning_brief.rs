//! Morning Brief — the day's first brief, in both the places it is delivered.
//!
//! Flow:
//! 1. Timer fires 3 seconds after boot
//! 2. If today's brief has not been delivered yet, the day is claimed and both halves go out:
//!    the structured card (active context sections, read straight from companion state) and
//!    the conversational brief in chat (an LLM round that uses the companion's tools)
//! 3. Populates the MorningBriefCard Slint properties
//! 4. Card auto-hides after 5 minutes or on user dismiss
//!
//! The card and the chat brief are two surfaces of one thing: the card is a visual summary,
//! the chat is the conversational greeting. They used to be triggered separately — the card
//! from here, guarded to once a day, and the chat one from `wire::timers`, guarded by nothing
//! at all. That one is an LLM round with tools, and the companion runs one thing at a time, so
//! every shell start put the machine's mind out of reach for the minutes it took: a tool call
//! or a question asked in that window simply waited. The shell restarts on every deploy and
//! every crash, so the mind was unavailable after each of them, for a brief nobody asked for a
//! second time. One trigger now, one guard, both halves behind it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::bridge::MorningBriefSnapshot;
use crate::{App, BriefSection};

/// Wire the morning brief card on the desktop.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_brief_card(ui, ctx);
    wire_brief_dismiss(ui);
    wire_brief_section_action(ui, ctx);
}

/// Where the dated marker lives — beside the rest of the shell's state. Deliberately not in
/// memory: the point is to survive the restart, which is the whole thing that was going wrong.
fn brief_stamp_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".local/share/yantrik/last-brief"))
}

/// Whether today's brief is still owed, given the local calendar day and what the stamp last
/// recorded. `None` is no stamp file — a machine that has never delivered one.
///
/// The decision is a function of its two inputs and nothing else, so it can be tested without
/// a clock, a home directory or a companion. Everything around it is I/O.
///
/// No time-of-day window, deliberately. The name says morning, but the design does not: the
/// card's greeting was changed in `bridge.rs` to ask the clock rather than assume ("this was
/// hardcoded to Good morning, so a machine booted at 20:29 greeted its user with a card saying
/// good morning"), which settles that a brief at 20:29 is expected to appear and to know what
/// hour it is. A window would instead mean a machine first switched on after lunch never gets a
/// brief at all, which is a worse failure than a well-timed greeting. The known edge is a
/// restart that straddles midnight: the day changes, so a second brief goes out. Once a day
/// across a boundary the user crossed themselves is the behaviour, not the fault being fixed.
fn brief_is_due(today: &str, stamp: Option<&str>) -> bool {
    match stamp {
        Some(recorded) => recorded.trim() != today,
        None => true,
    }
}

/// Take today's brief slot, returning false if it has already been taken.
fn claim_brief_for_today() -> bool {
    let Some(marker) = brief_stamp_path() else {
        // No HOME means nowhere to record this. Deliver, rather than go silent on a machine
        // whose environment is unusual.
        return true;
    };
    let today = crate::app_context::current_date_short();
    let stamp = std::fs::read_to_string(&marker).ok();

    if !brief_is_due(&today, stamp.as_deref()) {
        return false;
    }
    write_stamp(&marker, &today);
    true
}

/// Write the stamp through a temporary file and rename it into place.
///
/// The rename is the point: a shell killed mid-write — which is exactly what happens here, the
/// shell is restarted constantly — must not leave a half-written date that matches no day and
/// so lets the brief fire on every start again.
fn write_stamp(marker: &std::path::Path, today: &str) {
    let Some(dir) = marker.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    let tmp = marker.with_extension("tmp");
    // If the write fails the brief still shows; showing it twice is a smaller fault than
    // never showing it because a directory was read-only.
    if std::fs::write(&tmp, today).is_ok() {
        let _ = std::fs::rename(&tmp, marker);
    }
}

/// Timer: 3 seconds after boot, claim the day and deliver both halves of the brief.
fn wire_brief_card(ui: &App, ctx: &AppContext) {
    let bridge = ctx.bridge.clone();
    let user_name = ctx.user_name.clone();
    let ui_weak = ui.as_weak();

    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
        // On an installed machine this fires behind the login screen: it would claim the day,
        // run the LLM round and compose a brief nobody has signed in to see, and the reply
        // would arrive as a notification over the lock (#203). Dropped, not queued — and
        // dropped before the claim, so a shell the person restarts after signing in still
        // delivers the day's brief.
        if let Some(ui) = ui_weak.upgrade() {
            if crate::control::locked_screen(ui.get_current_screen()) {
                tracing::info!("Morning brief dropped — the desktop is waiting for the person to sign in");
                return;
            }
        }

        // Only show if companion is online
        if !bridge.is_online() {
            tracing::info!("Morning brief card skipped — companion offline");
            return;
        }

        // And only once a day.
        //
        // It fired three seconds after every boot, so a morning of restarts meant the brief
        // arrived over and over — and during this work the shell was restarted dozens of times
        // an hour. A daily brief that appears on the fourth restart before lunch is not a
        // briefing, it is a popup.
        if !claim_brief_for_today() {
            tracing::info!("Morning brief already delivered today");
            return;
        }

        // The card first: it is answered straight from companion state, so it is already in
        // the command queue ahead of the LLM round below and comes back at once.
        let reply_rx = bridge.request_morning_brief();
        let weak = ui_weak.clone();

        // Then the conversational half. This is the expensive one — an LLM round that calls
        // tools, minutes long, during which the single-threaded companion answers nothing
        // else — which is why it is behind the same once-a-day claim as the card rather than
        // on its own unguarded timer in `wire::timers`, where it used to be.
        //
        // The prompt carries the time because there is no morning window: the model is told
        // what hour it is rather than being told it is morning, the same correction the card's
        // greeting already got.
        tracing::info!("Composing the day's brief");
        let prompt = format!(
            "You just started up. It is {time}. Compose the day's first brief for {user_name}. \
             Use your tools to check email, calendar, weather, system status, \
             and recall recent topics of interest. Skip any sources that fail \
             or that {user_name} has asked you not to include. \
             Keep it natural and concise — a few flowing sentences, no bullet points.",
            time = crate::app_context::current_time_hhmm(),
        );
        // Fire-and-forget: the response flows through the normal notification path.
        let _rx = bridge.send_message(prompt);

        // Poll for the reply (brief should arrive almost instantly — no LLM call)
        let poll_timer = Timer::default();
        let poll_timer_slot: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
        let slot = poll_timer_slot.clone();
        let poll_count: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        poll_timer.start(TimerMode::Repeated, Duration::from_millis(200), move || {
            *poll_count.borrow_mut() += 1;

            // Give up after 30 seconds (150 polls × 200ms)
            if *poll_count.borrow() > 150 {
                tracing::warn!("Morning brief reply timed out");
                if let Some(t) = slot.borrow_mut().take() {
                    t.stop();
                }
                return;
            }

            if let Ok(snapshot) = reply_rx.try_recv() {
                populate_brief_card(&weak, &snapshot);
                // Auto-dismiss after 5 minutes
                schedule_auto_dismiss(weak.clone());
                // Stop polling
                if let Some(t) = slot.borrow_mut().take() {
                    t.stop();
                }
            }
        });
        *poll_timer_slot.borrow_mut() = Some(poll_timer);
    });
    std::mem::forget(timer);
}

/// Populate the Slint card with brief data.
fn populate_brief_card(ui_weak: &slint::Weak<App>, snapshot: &MorningBriefSnapshot) {
    let greeting = snapshot.greeting.clone();
    let sections: Vec<BriefSection> = snapshot
        .sections
        .iter()
        .map(|s| BriefSection {
            icon: SharedString::from(&s.icon),
            label: SharedString::from(&s.label),
            content: SharedString::from(&s.content),
            expanded: s.expanded,
            action_id: SharedString::from(&s.action_id),
        })
        .collect();

    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_morning_brief_greeting(SharedString::from(&greeting));
            ui.set_morning_brief_sections(ModelRc::new(VecModel::from(sections)));
            ui.set_morning_brief_visible(true);
            tracing::info!("Morning brief card displayed");
        }
    });
}

/// Schedule auto-dismiss of the brief card after 5 minutes.
fn schedule_auto_dismiss(ui_weak: slint::Weak<App>) {
    let timer = Timer::default();
    timer.start(TimerMode::SingleShot, Duration::from_secs(300), move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_morning_brief_visible(false);
            tracing::debug!("Morning brief card auto-dismissed");
        }
    });
    std::mem::forget(timer);
}

/// Wire the dismiss callback — hides the card.
fn wire_brief_dismiss(ui: &App) {
    let ui_weak = ui.as_weak();
    ui.on_morning_brief_dismissed(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_morning_brief_visible(false);
            tracing::info!("Morning brief card dismissed by user");
        }
    });
}

/// Wire section action callbacks — navigate to relevant app screens.
fn wire_brief_section_action(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let _bridge = ctx.bridge.clone();
    ui.on_morning_brief_section_action(move |action_id| {
        let action = action_id.to_string();
        if action.starts_with("navigate:") {
            let screen_name = &action["navigate:".len()..];
            let screen_id = match screen_name {
                "weather" => 19,
                "calendar" => 18,
                "email" => 17,
                "notifications" => 9,
                _ => return,
            };
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_current_screen(screen_id);
                tracing::info!(screen = screen_name, "Morning brief section navigated");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::brief_is_due;

    // `today` is whatever `app_context::current_date_short()` produces — "Sun 20 Sep". The
    // tests use that shape so a change to the format shows up here rather than as a brief that
    // fires every start again.

    #[test]
    fn a_machine_with_no_stamp_is_owed_a_brief() {
        assert!(brief_is_due("Sun 20 Sep", None));
    }

    #[test]
    fn a_stamp_from_today_means_it_was_already_delivered() {
        assert!(!brief_is_due("Sun 20 Sep", Some("Sun 20 Sep")));
    }

    #[test]
    fn restarts_through_the_day_do_not_deliver_it_again() {
        let today = "Sun 20 Sep";
        assert!(brief_is_due(today, None));
        // Every start after the first reads the stamp it wrote.
        for _ in 0..20 {
            assert!(!brief_is_due(today, Some(today)));
        }
    }

    #[test]
    fn a_stamp_from_yesterday_is_owed_a_brief() {
        assert!(brief_is_due("Sun 20 Sep", Some("Sat 19 Sep")));
    }

    #[test]
    fn a_stamp_with_a_trailing_newline_still_counts_as_today() {
        assert!(!brief_is_due("Sun 20 Sep", Some("Sun 20 Sep\n")));
    }

    #[test]
    fn an_empty_or_truncated_stamp_is_owed_a_brief() {
        // The case the temp-file-and-rename write exists to prevent. If one is found anyway,
        // it must not read as "delivered" — a lost brief is worse than a repeated one.
        assert!(brief_is_due("Sun 20 Sep", Some("")));
        assert!(brief_is_due("Sun 20 Sep", Some("Sun 2")));
    }
}
