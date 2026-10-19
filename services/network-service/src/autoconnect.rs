//! Keeping NetworkManager trying to rejoin a saved Wi-Fi network.
//!
//! # Why
//!
//! A Wi-Fi profile's `connection.autoconnect-retries` is `-1` unless somebody sets it, and `-1`
//! means NetworkManager's global default, which is four. After four failed attempts to rejoin by
//! itself, NetworkManager blocks autoconnect for that profile until something activates it by
//! hand or the machine restarts. On the 2012 Mac mini (a BCM4331 on the `wl` driver) the link
//! drops and the driver is slow to bring it back, so four attempts are spent while the adapter is
//! still finding its feet, and after that the machine sits off the network with its radio on and
//! its network in range. That is what "had to restart" was (14 October 2026).
//!
//! `0` means "retry forever". This module sets it on the Wi-Fi profiles this service joins, and
//! once at start on the saved Wi-Fi profiles that still carry the default. A value somebody chose
//! on purpose — any value but `-1` — is left as it is.
//!
//! # No secret here
//!
//! Joining a secured network hands the passphrase to nmcli on stdin ([`Runner::run_with_secret`]),
//! and nothing in this file builds a command line out of it. The tests below record every argv
//! this module runs and look for the password in each.

use crate::nmcli::{self, Exit, Runner, Trouble};

/// The value that means "never stop trying".
pub const RETRY_FOREVER: &str = "0";

/// What a profile's `connection.autoconnect-retries` holds, as nmcli prints it.
///
/// `-g` prints the bare value (`-1`), and some releases add a gloss (`-1 (default)`), so the first
/// word is the one read. Anything that is not a number is `None`: a value this cannot read is a
/// value it does not change.
pub fn parse_retries(text: &str) -> Option<i32> {
    text.split_whitespace().next()?.parse().ok()
}

/// Whether a profile still has NetworkManager's default number of retries.
pub fn is_default(retries: Option<i32>) -> bool {
    retries == Some(-1)
}

/// Read one profile's retries.
fn read_retries(runner: &dyn Runner, uuid: &str) -> Result<Option<i32>, Trouble> {
    let exit = runner.run(
        nmcli::QUICK_WAIT_SECS,
        &["-g", "connection.autoconnect-retries", "connection", "show", "uuid", uuid],
    );
    nmcli::outcome(&exit).map(|text| parse_retries(&text))
}

/// Set one profile to retry forever if it still has the default. `Ok(true)` when it was changed.
///
/// `modify uuid <uuid>`, never `modify <name>`: a Wi-Fi profile is named after its SSID, and an
/// SSID may begin with a dash or contain a colon. Changing this property on a profile that is up
/// does not take the link down; NetworkManager reads it the next time it decides whether to retry.
pub fn keep_trying(runner: &dyn Runner, uuid: &str) -> Result<bool, Trouble> {
    if !is_default(read_retries(runner, uuid)?) {
        return Ok(false);
    }
    let exit = runner.run(
        nmcli::QUICK_WAIT_SECS,
        &["connection", "modify", "uuid", uuid, "connection.autoconnect-retries", RETRY_FOREVER],
    );
    nmcli::outcome(&exit).map(|_| true)
}

/// The Wi-Fi profile that is up on `device`, as nmcli lists the active connections.
fn active_wifi_profile(runner: &dyn Runner, device: &str) -> Result<Option<nmcli::ActiveConnection>, Trouble> {
    let exit = runner.run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", nmcli::ACTIVE_FIELDS, "connection", "show", "--active"],
    );
    let text = nmcli::outcome(&exit)?;
    Ok(nmcli::parse_active(&text)
        .into_iter()
        .find(|c| c.device == device && c.kind == "802-11-wireless" && !c.uuid.is_empty()))
}

/// Join `ssid` on `device`, then make the profile it was joined with keep trying.
///
/// The join is the one `network.wifi_connect` always ran: `nmcli device wifi connect <ssid>`,
/// which uses the saved profile for that network when there is one and makes one when there is
/// not, with the password, when one is given, on stdin. What it returns is the join's outcome;
/// the retries setting is reported beside it and never turns a join that worked into a failure,
/// because the person is on the network either way.
pub fn connect(
    runner: &dyn Runner,
    device: &str,
    ssid: &str,
    password: Option<&str>,
) -> Result<KeptTrying, Trouble> {
    let argv = ["device", "wifi", "connect", ssid];
    let exit: Exit = match password.filter(|p| !p.is_empty()) {
        // `--ask` plus the secret on stdin. `… password <pw>` would leave the passphrase in
        // /proc/<pid>/cmdline for the twenty-five seconds a connect may run.
        Some(secret) => runner.run_with_secret(nmcli::CONNECT_WAIT_SECS, &argv, secret),
        // An open network, or one this machine already has credentials for.
        None => runner.run(nmcli::CONNECT_WAIT_SECS, &argv),
    };
    nmcli::outcome(&exit)?;

    Ok(match active_wifi_profile(runner, device) {
        Ok(Some(profile)) => match keep_trying(runner, &profile.uuid) {
            Ok(true) => KeptTrying::Set(profile.name),
            Ok(false) => KeptTrying::AlreadySet(profile.name),
            Err(trouble) => KeptTrying::Failed(trouble.message()),
        },
        Ok(None) => KeptTrying::Failed(format!("no Wi-Fi profile is up on {device} after the join")),
        Err(trouble) => KeptTrying::Failed(trouble.message()),
    })
}

/// What happened to the retries setting after a join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeptTrying {
    /// The profile (named) had the default and now retries forever.
    Set(String),
    /// The profile (named) already had a value somebody chose, which was left alone.
    AlreadySet(String),
    /// It could not be read or set; why, in words.
    Failed(String),
}

/// What the start-up pass did to one saved profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touched {
    pub name: String,
    pub result: Result<bool, String>,
}

/// Once, when the service starts: every saved Wi-Fi profile that still has the default is set to
/// retry forever. Profiles of any other type, and Wi-Fi profiles with a value of their own, are
/// not touched. Answers with one row per Wi-Fi profile looked at, for the log.
pub fn startup_pass(runner: &dyn Runner) -> Result<Vec<Touched>, Trouble> {
    let exit = runner.run(
        nmcli::QUICK_WAIT_SECS,
        &["-t", "-f", nmcli::CONNECTION_FIELDS, "connection", "show"],
    );
    let saved = nmcli::parse_known(&nmcli::outcome(&exit)?);
    Ok(saved
        .into_iter()
        .filter(|profile| !profile.uuid.is_empty())
        .map(|profile| Touched {
            result: keep_trying(runner, &profile.uuid).map_err(|t| t.message()),
            name: profile.ssid,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// nmcli as a script: each argv is recorded, and answered by the first rule whose words all
    /// appear in it. A command no rule answers fails the way nmcli fails on a bad argument, so a
    /// test cannot pass by running something it did not expect.
    struct FakeNmcli {
        rules: Vec<(Vec<&'static str>, &'static str)>,
        ran: RefCell<Vec<Vec<String>>>,
        stdin: RefCell<Vec<String>>,
    }

    impl FakeNmcli {
        fn new(rules: Vec<(Vec<&'static str>, &'static str)>) -> Self {
            FakeNmcli { rules, ran: RefCell::new(Vec::new()), stdin: RefCell::new(Vec::new()) }
        }

        fn answer(&self, argv: &[&str]) -> Exit {
            self.ran.borrow_mut().push(argv.iter().map(|s| s.to_string()).collect());
            match self.rules.iter().find(|(words, _)| words.iter().all(|w| argv.contains(w))) {
                Some((_, stdout)) => Exit::Ran { code: Some(0), stdout: stdout.to_string(), stderr: String::new() },
                None => Exit::Ran { code: Some(2), stdout: String::new(), stderr: format!("Error: unexpected {argv:?}") },
            }
        }

        /// Every `connection modify` this ran, as the uuid it named.
        fn modified(&self) -> Vec<String> {
            self.ran
                .borrow()
                .iter()
                .filter(|argv| argv.iter().any(|a| a == "modify"))
                .map(|argv| argv[argv.iter().position(|a| a == "uuid").unwrap() + 1].clone())
                .collect()
        }
    }

    impl Runner for FakeNmcli {
        fn run(&self, _wait: u32, argv: &[&str]) -> Exit {
            self.answer(argv)
        }

        fn run_with_secret(&self, _wait: u32, argv: &[&str], secret: &str) -> Exit {
            self.stdin.borrow_mut().push(secret.to_string());
            self.answer(argv)
        }
    }

    const ACTIVE: &str = "Home:aaaa-1111:802-11-wireless:wlp3s0\nWired connection 1:bbbb-2222:802-3-ethernet:eth0\n";

    #[test]
    fn joining_a_network_sets_its_profile_to_retry_forever() {
        let nm = FakeNmcli::new(vec![
            (vec!["device", "wifi", "connect", "Home"], "Device 'wlp3s0' successfully activated."),
            (vec!["--active"], ACTIVE),
            (vec!["-g", "connection.autoconnect-retries", "aaaa-1111"], "-1\n"),
            (vec!["modify", "aaaa-1111", "connection.autoconnect-retries", "0"], ""),
        ]);
        let kept = connect(&nm, "wlp3s0", "Home", Some("correct horse battery")).expect("joined");
        assert_eq!(kept, KeptTrying::Set("Home".into()));
        assert_eq!(nm.modified(), ["aaaa-1111"], "the Wi-Fi profile on the device, not the wired one");
    }

    #[test]
    fn the_password_reaches_nmcli_on_stdin_and_on_no_command_line() {
        let password = "correct horse battery";
        let nm = FakeNmcli::new(vec![
            (vec!["device", "wifi", "connect", "Home"], ""),
            (vec!["--active"], ACTIVE),
            (vec!["-g"], "-1"),
            (vec!["modify"], ""),
        ]);
        connect(&nm, "wlp3s0", "Home", Some(password)).expect("joined");
        assert_eq!(*nm.stdin.borrow(), [password], "handed over once, on stdin");
        for argv in nm.ran.borrow().iter() {
            assert!(!argv.iter().any(|a| a.contains(password)), "the password is on a command line: {argv:?}");
            assert!(!argv.iter().any(|a| a == "password"), "nmcli's `password` argument is used: {argv:?}");
        }
    }

    #[test]
    fn a_saved_network_is_joined_with_nothing_on_stdin() {
        let nm = FakeNmcli::new(vec![
            (vec!["device", "wifi", "connect", "Home"], ""),
            (vec!["--active"], ACTIVE),
            (vec!["-g"], "-1"),
            (vec!["modify"], ""),
        ]);
        connect(&nm, "wlp3s0", "Home", None).expect("joined");
        assert!(nm.stdin.borrow().is_empty());
    }

    #[test]
    fn a_retries_value_somebody_chose_is_left_as_it_is() {
        let nm = FakeNmcli::new(vec![
            (vec!["device", "wifi", "connect", "Home"], ""),
            (vec!["--active"], ACTIVE),
            (vec!["-g"], "3\n"),
        ]);
        assert_eq!(connect(&nm, "wlp3s0", "Home", None), Ok(KeptTrying::AlreadySet("Home".into())));
        assert!(nm.modified().is_empty());
    }

    /// The join is what the caller asked for. A retries setting that could not be made is said
    /// beside it, and does not turn a machine that is on the network into a failed call.
    #[test]
    fn a_join_that_worked_is_not_failed_by_the_retries_setting() {
        let nm = FakeNmcli::new(vec![
            (vec!["device", "wifi", "connect", "Home"], ""),
            (vec!["--active"], ACTIVE),
            (vec!["-g"], "-1"),
            // No rule for `modify`: nmcli refuses it.
        ]);
        assert!(matches!(connect(&nm, "wlp3s0", "Home", None), Ok(KeptTrying::Failed(_))));
    }

    #[test]
    fn a_join_that_failed_touches_no_profile() {
        let nm = FakeNmcli::new(vec![]);
        assert!(connect(&nm, "wlp3s0", "Home", Some("pw")).is_err());
        assert_eq!(nm.ran.borrow().len(), 1, "only the connect ran");
    }

    #[test]
    fn the_startup_pass_changes_only_wifi_profiles_that_still_have_the_default() {
        let nm = FakeNmcli::new(vec![
            (
                vec!["-f", nmcli::CONNECTION_FIELDS, "show"],
                "Home:aaaa-1111:802-11-wireless:yes\n\
                 Cafe\\: Free:cccc-3333:802-11-wireless:no\n\
                 Office:dddd-4444:802-11-wireless:no\n\
                 Wired connection 1:bbbb-2222:802-3-ethernet:yes\n",
            ),
            (vec!["-g", "aaaa-1111"], "-1\n"),
            (vec!["-g", "cccc-3333"], "-1 (default)\n"),
            (vec!["-g", "dddd-4444"], "0\n"),
            (vec!["modify"], ""),
        ]);
        let touched = startup_pass(&nm).expect("the pass ran");
        assert_eq!(nm.modified(), ["aaaa-1111", "cccc-3333"], "the two Wi-Fi profiles still on the default");
        assert_eq!(
            touched,
            [
                Touched { name: "Home".into(), result: Ok(true) },
                Touched { name: "Cafe: Free".into(), result: Ok(true) },
                Touched { name: "Office".into(), result: Ok(false) },
            ],
            "one row per Wi-Fi profile, none for the wired one"
        );
        assert!(
            !nm.ran.borrow().iter().any(|argv| argv.iter().any(|a| a == "bbbb-2222")),
            "the wired profile is not even read"
        );
    }

    #[test]
    fn a_retries_value_that_cannot_be_read_is_not_changed() {
        assert_eq!(parse_retries("-1"), Some(-1));
        assert_eq!(parse_retries("-1 (default)"), Some(-1));
        assert_eq!(parse_retries("0\n"), Some(0));
        assert_eq!(parse_retries(""), None);
        assert_eq!(parse_retries("--"), None);
        assert!(!is_default(None));
    }
}
