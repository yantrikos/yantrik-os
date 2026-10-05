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
//!   the old profile, and the reason is said ([`crate::power_status::settled`]).

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

/// Said when a toggle did not change, so it is seen whatever screen is up — the Settings save
/// line is only on Settings, and a tile is pressed from Quick Settings, Today and the bar.
pub(crate) fn say_refused(what: &str, why: &str) {
    tracing::warn!(%what, %why, "a toggle's backend refused the change");
    crate::wire::notifications::private_mode_notice(&format!("{what} did not change"), why);
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

    /// The handler's body, from `ui.on_<name>(` to the closing `});` at its own indent.
    fn handler<'a>(src: &'a str, name: &str) -> &'a str {
        let open = format!("ui.on_{name}(");
        let at = src.find(&open).unwrap_or_else(|| panic!("no {open}"));
        let indent = src[..at].rsplit('\n').next().unwrap_or_default();
        let end = src[at..].find(&format!("\n{indent}}});")).unwrap_or_else(|| panic!("{open} never closes"));
        &src[at..at + end]
    }

    /// Each toggle that answers on the UI thread writes first, through [`write_then_show`], and
    /// never sets its property before the write has answered.
    #[test]
    fn dnd_and_dark_style_show_only_what_was_written() {
        let src = include_str!("wire/settings.rs");
        for (name, prop) in [("toggle_dnd_mode", "ui.set_dnd_mode("), ("toggle_dark_mode", "ui.set_settings_dark_mode(")] {
            let body = handler(src, name);
            let write = body.find("toggle_truth::write_then_show(").unwrap_or_else(|| panic!("{name} flips before the write"));
            let shown = body.find(prop).unwrap_or_else(|| panic!("{name} never shows the result"));
            assert!(shown > write, "{name} sets {prop}…) before the backend answered");
            assert!(body.contains("toggle_truth::say_refused("), "{name} drops the refusal");
        }
    }

    #[test]
    fn a_skill_shows_what_the_database_took_and_says_when_it_did_not() {
        let body = handler(include_str!("wire/skill_store.rs"), "toggle_skill");
        assert!(body.contains("toggle_truth::say_refused("), "a refused skill toggle is silent");
        assert!(!body.contains("if let Ok(conn)"), "a database that will not open is dropped silently");
    }

    /// The power profile answers later: pending is shown before the daemon is asked, and cleared
    /// with what it reads back.
    #[test]
    fn the_power_profile_is_pending_until_the_daemon_answers() {
        let body = handler(include_str!("power_status.rs"), "set_power_profile");
        let pending = body.find("set_power_profile_pending(profile").expect("the tile is not marked pending");
        let asked = body.find("power_profile::set(").expect("the daemon is asked");
        assert!(pending < asked, "pending is shown only after the daemon was asked");
        assert!(body.contains("set_power_profile_pending(\"\".into())"), "pending is never cleared");
        assert!(body.contains("toggle_truth::say_refused("), "a refusal or timeout is silent");
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
