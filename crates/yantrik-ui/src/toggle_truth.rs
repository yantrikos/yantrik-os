//! A toggle shows what its backend confirmed, never what was asked.
//!
//! The outside design review: a tile that lights before the machine has done the thing is a
//! claim, and when the write fails the claim stays up. Do Not Disturb and Dark style flipped
//! first and saved after, dropping the error into a Settings line nobody opening Quick Settings
//! sees; a Skill flipped in memory and ignored the database. So each now goes one of two ways:
//!
//! - **Write, then show** ([`write_then_show`]), for a backend that answers on the UI thread (a
//!   file, a database). The write finishes before the next frame is drawn, so there is no
//!   unconfirmed frame to draw a pending state in: the tile goes from what was to what was
//!   confirmed, or stays, with the reason said.
//! - **Pending, then what the backend says**, for one that answers later (the power profile, a
//!   D-Bus call with a two-second timeout). The tile says "Switching…" and takes no press until the
//!   daemon answers; then it shows what the daemon reads back, which on a refusal or a timeout is
//!   the old profile, and the reason is said ([`crate::power_choice`]).

/// How a toggle came out: what to show, and why it is not what was asked when it is not.
#[derive(Debug, PartialEq)]
pub(crate) struct Settled<T> {
    pub shown: T,
    pub refused: Option<String>,
}

/// Ask the backend for `want`; show it only when the write succeeded, else stay at `was`.
pub(crate) fn write_then_show(was: bool, want: bool, write: impl FnOnce(bool) -> Result<(), String>) -> Settled<bool> {
    match write(want) {
        Ok(()) => Settled { shown: want, refused: None },
        Err(why) => Settled { shown: was, refused: Some(why) },
    }
}

/// A setting held in memory and saved to a file, flipped from `was`: `remember` sets the value in
/// memory, `write` saves it. A refused save puts the value in memory back to `was` (so the next
/// save of any other setting does not write the refused one after all), says why through `say`,
/// and answers `was`, which is what the tile then shows. Do Not Disturb's and Dark style's
/// handlers are this, with the real store and [`say_refused`].
pub(crate) fn flip_saved(
    what: &str,
    was: bool,
    remember: impl Fn(bool),
    write: impl FnOnce(bool) -> Result<(), String>,
    say: impl FnOnce(&str, &str),
) -> bool {
    let out = write_then_show(was, !was, |want| {
        remember(want);
        write(want).inspect_err(|_| remember(was))
    });
    if let Some(why) = &out.refused {
        say(what, why);
    }
    out.shown
}

/// Said when a toggle did not change, so it is seen whatever screen is up — the Settings save
/// line is only on Settings, and a tile is pressed from Quick Settings, Today and the bar.
pub(crate) fn say_refused(what: &str, why: &str) {
    tracing::warn!(%what, %why, "a toggle's backend refused the change");
    crate::wire::notifications::setting_not_saved(&format!("{what} did not change"), why);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_confirmed_write_shows_the_new_value() {
        assert_eq!(write_then_show(false, true, |_| Ok(())), Settled { shown: true, refused: None });
    }

    #[test]
    fn a_refused_write_stays_where_it_was_and_says_why() {
        let out = write_then_show(false, true, |_| Err("read-only file system".into()));
        assert_eq!(out, Settled { shown: false, refused: Some("read-only file system".into()) });
    }

    fn profiles(active: &str) -> yantrik_os::power_profile::PowerProfiles {
        let offered = ["power-saver", "balanced"].map(String::from).to_vec();
        yantrik_os::power_profile::PowerProfiles { active: active.into(), offered }
    }

    /// The daemon's answer is what the tile shows: taken, refused, timed out, or taken as
    /// something else.
    #[test]
    fn the_power_tile_shows_what_the_daemon_reads_back() {
        let taken = crate::power_status::settled("power-saver", Ok(profiles("power-saver")), || panic!("not read"));
        assert_eq!(taken, Settled { shown: Some(profiles("power-saver")), refused: None });

        let timed_out = crate::power_status::settled(
            "power-saver",
            Err("the daemon did not take it (it refused, or did not answer within 2 s)".into()),
            || Some(profiles("balanced")),
        );
        assert_eq!(timed_out.shown, Some(profiles("balanced")), "a timeout shows the profile still in effect");
        assert!(timed_out.refused.unwrap().contains("did not answer"));

        let gone = crate::power_status::settled("power-saver", Err("no system bus".into()), || None);
        assert_eq!(gone.shown, None, "no daemon left: the choice is not drawn");
        assert!(gone.refused.is_some());

        let other = crate::power_status::settled("performance", Ok(profiles("balanced")), || None);
        assert_eq!(other.shown, Some(profiles("balanced")));
        assert!(other.refused.unwrap().contains("`balanced`"), "taken as something else is a refusal");
    }

    /// A save that fails, through the same function the handlers use: the value in memory is back
    /// where it was, the refusal is said once, and the tile is told the old value.
    #[test]
    fn a_refused_save_restores_memory_says_why_and_claims_nothing() {
        for was in [false, true] {
            let memory = std::cell::Cell::new(was);
            let said = std::cell::RefCell::new(Vec::new());
            let shown = flip_saved(
                "Dark style",
                was,
                |v| memory.set(v),
                |want| {
                    assert_eq!(memory.get(), want, "the value is in memory when the save is made");
                    Err("read-only file system".into())
                },
                |what, why| said.borrow_mut().push(format!("{what} did not change: {why}")),
            );
            assert_eq!(shown, was, "the tile is told the old value");
            assert_eq!(memory.get(), was, "the refused value is not left in memory for the next save");
            assert_eq!(*said.borrow(), ["Dark style did not change: read-only file system"]);
        }
    }

    #[test]
    fn a_taken_save_shows_the_new_value_and_says_nothing() {
        let memory = std::cell::Cell::new(false);
        let shown = flip_saved("Do Not Disturb", false, |v| memory.set(v), |_| Ok(()), |w, why| panic!("{w}: {why}"));
        assert!(shown && memory.get());
    }

    /// The handler's body, from `ui.on_<name>(` to the closing `});` at its own indent.
    fn handler<'a>(src: &'a str, name: &str) -> &'a str {
        let open = format!("ui.on_{name}(");
        let at = src.find(&open).unwrap_or_else(|| panic!("no {open}"));
        let indent = src[..at].rsplit('\n').next().unwrap_or_default();
        let end = src[at..].find(&format!("\n{indent}}});")).unwrap_or_else(|| panic!("{open} never closes"));
        &src[at..at + end]
    }

    /// Do Not Disturb and Dark style are [`flip_saved`] with the real store, which the test above
    /// runs with a failing one; the handler shows only what it answered.
    #[test]
    fn dnd_and_dark_style_go_through_flip_saved() {
        let src = include_str!("wire/settings.rs");
        for (name, prop) in [("toggle_dnd_mode", "ui.set_dnd_mode(shown)"), ("toggle_dark_mode", "ui.set_settings_dark_mode(shown)")] {
            let body = handler(src, name);
            assert!(body.contains("toggle_truth::flip_saved("), "{name} does not go through flip_saved");
            assert!(body.contains("toggle_truth::say_refused"), "{name} drops the refusal");
            assert!(body.contains(prop), "{name} shows something other than what was saved");
        }
    }

    #[test]
    fn a_skill_shows_what_the_database_took_and_says_when_it_did_not() {
        let body = handler(include_str!("wire/skill_store.rs"), "toggle_skill");
        assert!(body.contains("toggle_truth::say_refused("), "a refused skill toggle is silent");
        assert!(!body.contains("if let Ok(conn)"), "a database that will not open is dropped silently");
    }

    /// Private already wrote first; this keeps it so.
    #[test]
    fn private_shows_what_its_file_says() {
        let src = include_str!("control_approvals.rs");
        let body = handler(src, "mind_private_chosen");
        let write = body.find("person_set_private(").expect("Private writes its file");
        let shown = body.find("set_private_mode(crate::private_mode::is_on())").expect("Private shows the file's word");
        assert!(shown > write);
        assert!(body.contains("private_mode_notice("), "a refusal is said");
    }
}
