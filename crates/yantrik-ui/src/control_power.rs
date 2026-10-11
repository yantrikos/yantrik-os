//! The power menu's acts, for the person's pointer and for a caller alike.
//!
//! The bar's power popover (`components/power_menu.slint`) offers Lock, Log out, Suspend,
//! Hibernate, Restart and Shut down. Lock already has its own `safe` action (`lock`: it only
//! takes access away). The other five are here, split by what they cost so each carries a grade
//! that is true:
//!
//!   * `power_sleep how=suspend|hibernate` is `sensitive`. The session survives, nothing is lost,
//!     but the machine stops answering until someone wakes it, and a mind that sleeps its own
//!     host has cut itself off.
//!   * `power_off how=logout|restart|shutdown` is `dangerous`. Each ends every program in the
//!     session, and with them any minds working and anything unsaved. It refuses while minds are
//!     working unless the caller says `even_if_minds_working=true`, which is the popover's "stop
//!     them" said in words; the answer to a refusal names how many.
//!
//! Both are deferred: the machine acts after the answer, so the answer is "logind accepted it",
//! never "it is off". Neither raises a window over the shell, so neither calls `hold_windows`;
//! opening the popover (`open_power_menu`) does, and shows the same confirmation a person gets.
//!
//! The popover's own buttons reach the same `run` through `wire::power`, so the pointer and a
//! caller cannot disagree about what "Restart" runs.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{answer_later, Action, App as ControlSurface, Param};

use crate::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Logout,
    Suspend,
    Hibernate,
    Restart,
    Shutdown,
}

impl Verb {
    pub fn parse(text: &str) -> Option<Verb> {
        Some(match text {
            "logout" => Verb::Logout,
            "suspend" => Verb::Suspend,
            "hibernate" => Verb::Hibernate,
            "restart" => Verb::Restart,
            "shutdown" => Verb::Shutdown,
            _ => return None,
        })
    }

    /// Whether this ends the session's programs, so a mind at work and unsaved files go with it.
    pub fn ends_the_session(self) -> bool {
        matches!(self, Verb::Logout | Verb::Restart | Verb::Shutdown)
    }

    /// The program and arguments that do it. Restart and power off go through logind with
    /// systemctl (the account at the active seat may, without sudo, #397); log out ends this
    /// login session by id.
    fn command(self) -> Result<(&'static str, Vec<String>), String> {
        Ok(match self {
            Verb::Suspend => ("systemctl", vec!["suspend".into()]),
            Verb::Hibernate => ("systemctl", vec!["hibernate".into()]),
            Verb::Restart => ("systemctl", vec!["reboot".into()]),
            Verb::Shutdown => ("systemctl", vec!["poweroff".into()]),
            Verb::Logout => {
                let id = std::env::var("XDG_SESSION_ID").map_err(|_| "this shell was not started in a login session, so there is none to end".to_string())?;
                ("loginctl", vec!["terminate-session".into(), id])
            }
        })
    }
}

/// Do it. Blocks until logind has accepted or refused, so run it on a worker or inside
/// `answer_later`, never on the thread that draws.
pub fn run(verb: Verb) -> Result<(), String> {
    if verb == Verb::Hibernate && !yantrik_os::login1::can_hibernate() {
        return Err("logind does not offer hibernate on this machine (no swap that can hold memory, or no resume device)".to_string());
    }
    let (program, args) = verb.command()?;
    match std::process::Command::new(program).args(&args).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!("{program} refused: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => Err(format!("could not run {program}: {e}")),
    }
}

/// How many different minds are working now, from the Agents store: the number the popover
/// confirms with and the one a refusal quotes.
pub fn minds_working() -> usize {
    crate::agents::store().read(|s| crate::mind_panel::working(s, 0, 0).minds)
}

/// "2 minds are working", or `None` when nobody is. Said the same way in the popover (which gets
/// the number from `power-minds-working`) and in a caller's refusal.
pub fn working_words(n: usize) -> Option<String> {
    match n {
        0 => None,
        1 => Some("1 mind is working".to_string()),
        n => Some(format!("{n} minds are working")),
    }
}

/// `describe shell`'s `power` field: what the popover offers, read from the same place it reads.
pub fn for_describe(ui: &App) -> serde_json::Value {
    serde_json::json!({
        "hibernate_offered": ui.get_power_can_hibernate(),
        "minds_working": minds_working(),
    })
}

pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let sleep_weak = ui.as_weak();
    let off_weak = ui.as_weak();
    surface
        .action(
            Action::new(
                "power_sleep",
                "Suspend the machine, or hibernate it where logind offers that (`describe shell` \
                 says under `power.hibernate_offered`). The session is kept; the machine stops \
                 answering until the person wakes it. Answers when logind has accepted the \
                 request, which is before the machine sleeps",
            )
            .risk("sensitive")
            .defers()
            .arg(Param::text("how").describe("suspend or hibernate")),
            move |args| {
                sleep_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let how = args["how"].as_str().unwrap_or_default().to_string();
                let verb = match Verb::parse(&how) {
                    Some(v @ (Verb::Suspend | Verb::Hibernate)) => v,
                    _ => return Err("`how` must be suspend or hibernate".to_string()),
                };
                let work = move || {
                    run(verb)?;
                    tracing::info!(how = %how, "Sleep requested through the control surface");
                    Ok(serde_json::json!({ "requested": how, "note": "logind accepted the request" }))
                };
                answer_later(work).map(|()| serde_json::json!({ "answering": "off the UI thread" })).or_else(|work| work())
            },
        )
        .action(
            Action::new(
                "power_off",
                "Log out, restart or shut down. Each ends every program in the session, minds \
                 working and unsaved files included, so it is refused while minds are working \
                 unless `even_if_minds_working` is true; the refusal says how many. Answers when \
                 logind has accepted the request, which is before anything has stopped",
            )
            .risk("dangerous")
            .defers()
            .arg(Param::text("how").describe("logout, restart or shutdown"))
            .arg(Param::flag("even_if_minds_working").describe("true to go ahead although minds are working; they are stopped").optional()),
            move |args| {
                off_weak.upgrade().ok_or_else(|| "the shell is gone".to_string())?;
                let how = args["how"].as_str().unwrap_or_default().to_string();
                let verb = match Verb::parse(&how) {
                    Some(v) if v.ends_the_session() => v,
                    _ => return Err("`how` must be logout, restart or shutdown".to_string()),
                };
                let forced = args["even_if_minds_working"].as_bool().unwrap_or(false);
                if let (Some(words), false) = (working_words(minds_working()), forced) {
                    return Err(format!(
                        "{how} was not run: {words}, and they would be stopped. Let them finish, or \
                         ask again with even_if_minds_working=true"
                    ));
                }
                let work = move || {
                    run(verb)?;
                    tracing::warn!(how = %how, forced, "Session end requested through the control surface");
                    Ok(serde_json::json!({ "requested": how, "note": "logind accepted the request" }))
                };
                answer_later(work).map(|()| serde_json::json!({ "answering": "off the UI thread" })).or_else(|work| work())
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = include_str!("control_power.rs");

    fn action(name: &str) -> &'static str {
        let from = SOURCE.find(&format!("\"{name}\",\n")).unwrap_or_else(|| panic!("{name} is published"));
        let rest = &SOURCE[from..];
        &rest[..rest.find("answer_later(work)").expect("the action answers through answer_later")]
    }

    /// The grades are the point: a sleep is `sensitive`, ending the session is `dangerous`, and
    /// neither may be `safe` because both leave the person without the screen they were using.
    #[test]
    fn sleep_is_sensitive_and_ending_the_session_is_dangerous_and_both_are_deferred() {
        let sleep = action("power_sleep");
        assert!(sleep.contains(".risk(\"sensitive\")") && sleep.contains(".defers()"), "{sleep}");
        let off = action("power_off");
        assert!(off.contains(".risk(\"dangerous\")") && off.contains(".defers()"), "{off}");
    }

    /// Nothing on the socket may end the session while minds are working unless it says so.
    #[test]
    fn power_off_refuses_while_minds_work_unless_told_to_stop_them() {
        let off = action("power_off");
        let refuse = off.find("working_words(minds_working())").expect("it reads the real count");
        let run_at = off.find("run(verb)?").expect("it runs the verb");
        assert!(refuse < run_at, "the check comes before anything runs");
        assert!(off.contains("even_if_minds_working"));
    }

    /// The popover's buttons and a caller run the same command; `sleep` cannot be asked to end
    /// the session by a different spelling, and the other way round.
    #[test]
    fn each_action_takes_only_its_own_verbs() {
        assert_eq!(Verb::parse("restart"), Some(Verb::Restart));
        assert_eq!(Verb::parse("lock"), None, "lock has its own action");
        assert_eq!(Verb::parse("Shutdown"), None);
        assert!(!Verb::Suspend.ends_the_session() && !Verb::Hibernate.ends_the_session());
        assert!(Verb::Logout.ends_the_session() && Verb::Restart.ends_the_session() && Verb::Shutdown.ends_the_session());
        let (program, args) = Verb::Restart.command().unwrap();
        assert_eq!((program, args), ("systemctl", vec!["reboot".to_string()]));
        let (_, args) = Verb::Shutdown.command().unwrap();
        assert_eq!(args, ["poweroff"], "systemd's name for power off is not \"shutdown\"");
    }

    #[test]
    fn the_count_is_said_in_words_and_silent_at_zero() {
        assert_eq!(working_words(0), None);
        assert_eq!(working_words(1).as_deref(), Some("1 mind is working"));
        assert_eq!(working_words(2).as_deref(), Some("2 minds are working"));
    }

    /// Nothing here puts a window over the shell, so nothing asks `hold_windows`; the day one does,
    /// this fails and says to add it.
    #[test]
    fn nothing_here_raises_a_window() {
        for call in ["raise_shell", "set_quick_settings_open", "set_power_menu_open", "invoke_lock_screen"] {
            assert!(!SOURCE[..SOURCE.find("#[cfg(test)]").unwrap()].contains(call), "{call} raises something over the shell: call card_watch::hold_windows first");
        }
    }
}
