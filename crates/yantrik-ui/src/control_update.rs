//! The machine can update itself, and an agent can ask it to.
//!
//! `yantrik-update` (a script shipped in `/opt/yantrik/bin`) does the real work: read the release
//! manifest, download the channel's bundle, verify its sha256, back up the current binaries, swap
//! them in, restart the shell, and roll back if the new shell does not answer. This exposes three
//! of its verbs on the shell's control surface so the whole thing is drivable the same way
//! everything else in this OS is — without a terminal and without a person.
//!
//! `check_update` is safe: it only reads. `set_update_channel` is `sensitive`, because it changes
//! which software this machine will install next. `apply_update` is `dangerous` and defers,
//! because it replaces the running system and then restarts it — the caller must watch the
//! version settle, not treat the call as the update.
//!
//! ── This file is also the only place the shell talks to the updater ──
//!
//! The About screen used to carry a SECOND update checker of its own (`wire/version.rs`): a
//! hardcoded plain-http fetch of the manifest, a hardcoded channel of `stable`, and a read of
//! `channels[ch]["components"][name]["version"]` — a shape no manifest this project has ever
//! published. Every failure path in it returned "no update", so the only answer that screen
//! could give was "All components up to date", including on a machine that could not resolve
//! the host. A fabricated success is worse than an error.
//!
//! So the locating, the running and the parsing all live here, once, and the About screen calls
//! into them. Two callers, one path, one set of sentences.

use std::collections::HashMap;
use std::process::Command;

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::App;

/// The channels this build knows how to name. The script validates the same list; it is short
/// enough that two copies are cheaper than a round trip to ask, and a name that gets past this
/// is still refused by the one writer.
pub const CHANNELS: [&str; 3] = ["nightly", "beta", "stable"];

/// Where the updater lives: beside this binary. The shell is started from `/opt/yantrik/bin`, and
/// nothing puts that on `PATH`, so a bare `Command::new("yantrik-update")` would ENOENT on a
/// clean install — the same trap the app launcher already learned.
pub fn update_bin() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("yantrik-update")))
        .unwrap_or_else(|| std::path::PathBuf::from("yantrik-update"))
}

/// What one run of the updater produced.
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run the updater and wait. Blocking — callers on the UI thread put it on a worker.
pub fn run_updater(args: &[&str]) -> Result<Run, String> {
    let bin = update_bin();
    if !bin.exists() {
        return Err(format!(
            "no updater at {} — this machine cannot check for updates",
            bin.display()
        ));
    }
    let out = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| format!("could not run the updater: {e}"))?;
    Ok(Run {
        // -1 for a signal death. It is not a code the script can return, so it cannot be
        // confused with one, and `parse_check` treats anything it does not recognise as an error.
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// Start `apply` detached and return immediately.
///
/// Detached in its own session with `setsid`, because the very next thing it does is kill this
/// shell. A child in the shell's process group would die with it, mid swap, which is the one way
/// this operation could brick the machine.
///
/// `force` and `allow_downgrade` are two different permissions and stay two arguments. `force`
/// means "this is the same build, install it anyway"; `allow_downgrade` means "I know the
/// channel's build is older than what is here". The updater refuses a downgrade without the
/// second one, so a caller cannot get a rollback out of a flag that never said rollback.
pub fn spawn_apply(
    channel: Option<&str>,
    force: bool,
    allow_downgrade: bool,
) -> Result<(), String> {
    let bin = update_bin();
    if !bin.exists() {
        return Err(format!(
            "no updater at {} — this machine cannot update itself",
            bin.display()
        ));
    }
    let mut cmd = Command::new("setsid");
    cmd.arg(&bin).arg("apply");
    if let Some(c) = channel {
        cmd.args(["--channel", c]);
    }
    if force {
        cmd.arg("--force");
    }
    if allow_downgrade {
        cmd.arg("--allow-downgrade");
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the updater: {e}"))
}

/// Strip ANSI colour so the updater's human output is clean JSON when it travels over the socket.
pub fn plain(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut s = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                // Skip a CSI escape: ESC [ ... letter.
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                s.push(c);
            }
        }
        let s = s.trim().to_string();
        if !s.is_empty() {
            out.push(s);
        }
    }
    out
}

/// `key=value` lines from `--porcelain`. Anything that is not one is skipped: the script's
/// porcelain carries nothing else, and a line we cannot read is not a line to guess about.
pub fn parse_kv(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                map.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    map
}

/// What `yantrik-update check --porcelain` said, as something the UI can switch on.
///
/// The point of this type is the thing it cannot express: there is no variant that means
/// "something went wrong, call it current". `up_to_date` is reachable only from an explicit
/// `result=up_to_date` on exit 0, and every other shape of input lands in an error variant
/// carrying the script's own sentence. `Ahead` is reachable only from an explicit
/// `result=ahead_of_channel` for the same reason: two builds the updater cannot put in order
/// are offered as an update, never reported as being behind this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The installed build is the channel's build.
    UpToDate { installed: String, git: String },
    /// The channel has a build this machine does not.
    UpdateAvailable {
        from: String,
        to: String,
        reason: String,
    },
    /// The channel's build is OLDER than what is installed: a developer deploy from main, a beta
    /// tester moved back to nightly, a bundle that arrived from a mirror behind the one this
    /// machine follows. There is nothing to install, and this variant is why the screen can say
    /// so — before it existed, any difference between two commits read as an update, so the
    /// About screen offered a build 150 commits older as one and put an Install button under it.
    /// Moving to it anyway is a downgrade, which `apply_update` needs telling about explicitly.
    Ahead {
        installed: String,
        channel_build: String,
        channel: String,
        reason: String,
    },
    /// The server answered and has nothing on this channel. The person can act on this: it is
    /// the one failure that a channel change fixes, which is why it is not folded into the next.
    NotPublished { channel: String, reason: String },
    /// The server did not answer.
    Unreachable { reason: String },
    /// Anything else, including an updater that is missing, too old to know `--porcelain`, or
    /// killed. Never rendered as good news.
    Error { reason: String },
}

impl CheckOutcome {
    /// One sentence for the screen.
    pub fn headline(&self) -> String {
        match self {
            Self::UpToDate { installed, git } => {
                let build = if git.is_empty() {
                    installed.clone()
                } else {
                    format!("{installed} ({git})")
                };
                if build.trim().is_empty() {
                    "Up to date.".to_string()
                } else {
                    format!("Up to date — {build}.")
                }
            }
            Self::UpdateAvailable { from, to, reason } => {
                if from.is_empty() || to.is_empty() {
                    format!("Update available — {reason}")
                } else {
                    format!("Update available — {from} → {to}")
                }
            }
            Self::Ahead { installed, channel_build, channel, reason } => {
                // The sentence names both builds. "Nothing to update" on its own is what a
                // machine that could not be checked used to sound like, and the interesting
                // fact here is the direction: the channel is behind, not level with, this one.
                if reason.is_empty() {
                    format!(
                        "Nothing to update — this machine is ahead of channel '{channel}': \
                         installed {installed}, the channel has {channel_build}"
                    )
                } else {
                    format!("Nothing to update — {reason}")
                }
            }
            Self::NotPublished { reason, .. } => format!("Could not check — {reason}"),
            Self::Unreachable { reason } => format!("Could not check — {reason}"),
            Self::Error { reason } => format!("Could not check — {reason}"),
        }
    }

    /// Is there something to install? Only ever true from an explicit update_available — and
    /// notably false for a channel that is behind this machine, where the Install button this
    /// drives used to appear as well, and pressing it rolled the machine back to an older build
    /// while calling that an update.
    pub fn can_install(&self) -> bool {
        matches!(self, Self::UpdateAvailable { .. })
    }

    /// The word the Slint side switches its colour and buttons on.
    pub fn state(&self) -> &'static str {
        match self {
            Self::UpToDate { .. } => "up-to-date",
            Self::UpdateAvailable { .. } => "available",
            // Its own word rather than "up-to-date", because the machine is not on the
            // channel's build; it is past it. Same consequence on screen — no Install button —
            // and a different fact.
            Self::Ahead { .. } => "ahead",
            Self::NotPublished { .. } => "not-published",
            Self::Unreachable { .. } => "failed",
            Self::Error { .. } => "failed",
        }
    }
}

/// Turn one run of `check --porcelain` into an outcome.
///
/// Pure, so the rule that matters is testable: no input that is not an explicit success is ever
/// read as one. The old checker in `wire/version.rs` had that rule inverted — its error handling
/// was `return no_update` — and the screen said "All components up to date" on a machine with no
/// DNS.
pub fn parse_check(code: i32, stdout: &str, stderr: &str) -> CheckOutcome {
    let kv = parse_kv(stdout);
    let get = |k: &str| kv.get(k).cloned().unwrap_or_default();
    let reason = get("reason");

    match kv.get("result").map(String::as_str) {
        Some("up_to_date") => {
            if code != 0 {
                // The script says current and then exits non-zero. That is not a machine to
                // call current; it is a script disagreeing with itself.
                return CheckOutcome::Error {
                    reason: format!(
                        "the updater reported 'up to date' but exited {code} — refusing to \
                         report a result it does not agree with itself about"
                    ),
                };
            }
            CheckOutcome::UpToDate {
                installed: get("installed_version"),
                git: get("installed_git"),
            }
        }
        Some("update_available") => CheckOutcome::UpdateAvailable {
            from: get("installed_git"),
            to: get("channel_git"),
            reason: if reason.is_empty() {
                "the channel has a build this machine does not".to_string()
            } else {
                reason
            },
        },
        Some("ahead_of_channel") => CheckOutcome::Ahead {
            installed: build_label(&get("installed_version"), &get("installed_git")),
            channel_build: build_label(&get("channel_version"), &get("channel_git")),
            channel: get("channel"),
            reason,
        },
        Some("not_published") => CheckOutcome::NotPublished {
            channel: get("channel"),
            reason: if reason.is_empty() {
                format!("channel '{}' is not published", get("channel"))
            } else {
                reason
            },
        },
        Some("unreachable") => CheckOutcome::Unreachable {
            reason: if reason.is_empty() {
                "the release server did not answer".to_string()
            } else {
                reason
            },
        },
        Some(other) => CheckOutcome::Error {
            reason: format!("the updater returned a result this build does not know: '{other}'"),
        },
        // No `result=` line at all: the updater is missing, is an older copy that predates
        // --porcelain, or died. Whatever it is, it is not evidence that the machine is current.
        None => {
            let said = plain(stderr)
                .into_iter()
                .chain(plain(stdout))
                .next_back()
                .unwrap_or_default();
            CheckOutcome::Error {
                reason: if said.is_empty() {
                    format!("the updater said nothing and exited {code}")
                } else {
                    said
                },
            }
        }
    }
}

/// What `yantrik-update status --porcelain` says about this machine's configuration.
///
/// Asked of the script rather than read out of `/opt/yantrik/update.conf` here, deliberately.
/// The script already parses that file, and a second parser in the shell is how the machine
/// came to have three different answers to "which channel is this".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateStatus {
    pub channel: String,
    /// `update.conf`, `BUILD`, `default` or `--channel`: where the channel came from.
    pub channel_source: String,
    pub host: String,
    pub scheme: String,
    pub version: String,
    pub git: String,
    pub conf: String,
    /// Can this process change the channel without sudo? The picker says so plainly rather
    /// than offering a control that will fail.
    pub conf_writable: bool,
}

/// "0.4.2 (deadbee)", or whatever part of that is known, or "unknown". One function because
/// both ends of a comparison are labelled the same way, and a screen showing "0.4.2 (deadbee)"
/// next to "deadbee" would make two builds of the same commit look like two different things.
pub fn build_label(version: &str, git: &str) -> String {
    match (version.is_empty(), git.is_empty()) {
        (true, true) => "unknown".to_string(),
        (false, true) => version.to_string(),
        (true, false) => git.to_string(),
        (false, false) => format!("{version} ({git})"),
    }
}

impl UpdateStatus {
    /// "0.4.2 (deadbee)", or whatever part of that is known.
    pub fn installed_label(&self) -> String {
        build_label(&self.version, &self.git)
    }
}

pub fn parse_status(stdout: &str) -> UpdateStatus {
    let kv = parse_kv(stdout);
    let get = |k: &str| kv.get(k).cloned().unwrap_or_default();
    UpdateStatus {
        channel: get("channel"),
        channel_source: get("channel_source"),
        host: get("host"),
        scheme: get("scheme"),
        version: get("version"),
        git: get("git"),
        conf: get("conf"),
        conf_writable: get("conf_writable") == "yes",
    }
}

/// Read this machine's update configuration. Blocking.
pub fn read_status() -> Result<UpdateStatus, String> {
    let run = run_updater(&["status", "--porcelain"])?;
    let status = parse_status(&run.stdout);
    if status.channel.is_empty() {
        let said = plain(&run.stderr)
            .into_iter()
            .next_back()
            .unwrap_or_else(|| format!("exit {}", run.code));
        return Err(format!("the updater could not say which channel this machine is on: {said}"));
    }
    Ok(status)
}

/// Run one check. Blocking.
pub fn check_now(channel: Option<&str>) -> CheckOutcome {
    let mut args: Vec<&str> = vec!["check", "--porcelain"];
    if let Some(c) = channel {
        args.push("--channel");
        args.push(c);
    }
    match run_updater(&args) {
        Ok(run) => parse_check(run.code, &run.stdout, &run.stderr),
        Err(e) => CheckOutcome::Error { reason: e },
    }
}

/// Change the channel through the script, which is the one writer of `update.conf`. Blocking.
pub fn set_channel(channel: &str) -> Result<UpdateStatus, String> {
    let channel = channel.trim();
    if !CHANNELS.contains(&channel) {
        return Err(format!(
            "'{channel}' is not a channel (nightly, beta or stable)"
        ));
    }
    let run = run_updater(&["set-channel", channel])?;
    if run.code != 0 {
        let said = plain(&run.stderr)
            .into_iter()
            .chain(plain(&run.stdout))
            .next_back()
            .unwrap_or_else(|| format!("the updater exited {}", run.code));
        return Err(said);
    }
    // Re-read rather than assume. The answer to "which channel is this machine on" is whatever
    // the file says afterwards, and a write that silently did not take is exactly the failure
    // this whole change exists to stop reporting as success.
    read_status()
}

/// Add the update actions to the shell's control surface.
pub fn actions(surface: ControlSurface, _ui: &App) -> ControlSurface {
    surface
        .action(
            Action::new(
                "check_update",
                "Ask the release server whether a newer build is available",
            )
            .risk("safe")
            .arg(
                Param::text("channel")
                    .describe("Release channel: stable, beta, or nightly (default: the configured one)")
                    .optional(),
            ),
            move |args| {
                let channel = args["channel"]
                    .as_str()
                    .map(str::trim)
                    .filter(|c| !c.is_empty());
                let outcome = check_now(channel);
                let status = read_status().ok();
                let base = serde_json::json!({
                    "channel": status.as_ref().map(|s| s.channel.clone()),
                    "host": status.as_ref().map(|s| s.host.clone()),
                    "installed": status.as_ref().map(|s| s.installed_label()),
                    "detail": outcome.headline(),
                });
                let mut base = base;
                let obj = base.as_object_mut().expect("json object");
                match &outcome {
                    CheckOutcome::UpToDate { .. } => {
                        obj.insert("result".into(), "up_to_date".into());
                        obj.insert("update_available".into(), false.into());
                        obj.insert("up_to_date".into(), true.into());
                        Ok(base)
                    }
                    CheckOutcome::UpdateAvailable { from, to, .. } => {
                        obj.insert("result".into(), "update_available".into());
                        obj.insert("update_available".into(), true.into());
                        obj.insert("up_to_date".into(), false.into());
                        obj.insert("from".into(), from.clone().into());
                        obj.insert("to".into(), to.clone().into());
                        Ok(base)
                    }
                    // An answer, not a failure: the caller asked what the channel has, and the
                    // channel is behind this machine. `up_to_date` stays false, because this
                    // build did not come from that channel — the true part is that there is
                    // nothing to install, and `apply_update` refuses to install it anyway
                    // without an explicit allow_downgrade.
                    CheckOutcome::Ahead { channel_build, .. } => {
                        obj.insert("result".into(), "ahead_of_channel".into());
                        obj.insert("update_available".into(), false.into());
                        obj.insert("up_to_date".into(), false.into());
                        obj.insert("ahead_of_channel".into(), true.into());
                        obj.insert("channel_build".into(), channel_build.clone().into());
                        Ok(base)
                    }
                    // An unpublished channel is a fact about the server, not a failure of the
                    // call: the caller asked what the channel has, and the answer is nothing.
                    // Returned as an answer so an agent can act on it — the fix is
                    // `set_update_channel`, not a retry.
                    CheckOutcome::NotPublished { channel, reason } => {
                        obj.insert("result".into(), "not_published".into());
                        obj.insert("update_available".into(), false.into());
                        obj.insert("up_to_date".into(), false.into());
                        obj.insert("unpublished_channel".into(), channel.clone().into());
                        obj.insert("detail".into(), reason.clone().into());
                        Ok(base)
                    }
                    CheckOutcome::Unreachable { reason } | CheckOutcome::Error { reason } => {
                        Err(format!("update check failed: {reason}"))
                    }
                }
            },
        )
        .action(
            // Sensitive rather than safe: it decides which software this machine installs next,
            // and nothing it writes takes effect until an `apply` — so it is not dangerous
            // either. It writes through the script, which is the single owner of update.conf.
            Action::new(
                "set_update_channel",
                "Choose which release channel this machine follows (nightly, beta or stable)",
            )
            .risk("sensitive")
            .arg(
                Param::text("channel")
                    .describe("nightly, beta or stable"),
            ),
            move |args| {
                let channel = args["channel"]
                    .as_str()
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .ok_or_else(|| "which channel? (nightly, beta or stable)".to_string())?;
                // The answer is the channel read back afterwards, not the channel that was
                // asked for. A write that did not take must not report the value it wanted.
                let status = set_channel(channel)?;
                Ok(serde_json::json!({
                    "channel": status.channel,
                    "channel_source": status.channel_source,
                    "host": status.host,
                    "scheme": status.scheme,
                    "conf": status.conf,
                    "next": "run check_update to see what that channel has; a channel with no \
                             builds on it answers 'not published', which is an answer and not \
                             an error",
                }))
            },
        )
        .action(
            // Dangerous and deferred, both meant. It replaces every binary on the machine and
            // restarts the shell you are talking to; the socket you asked on goes away and comes
            // back. The updater verifies the download and rolls back a shell that will not start,
            // so the machine cannot be left dark — but the caller still must watch the build
            // settle rather than believe the launch was the landing.
            Action::new(
                "apply_update",
                "Download, verify, and install the channel's latest build, then restart",
            )
            .risk("dangerous")
            .defers()
            .arg(
                Param::text("channel")
                    .describe("Release channel to install from (default: the configured one)")
                    .optional(),
            )
            .arg(
                Param::flag("force")
                    .describe("Reinstall even if the channel build matches what is installed")
                    .optional(),
            )
            .arg(
                // Not folded into `force`. That flag says "the same build, again"; this one says
                // "an older build, on purpose", and a caller that has to name the second thing
                // cannot do it by reaching for the first. Without it the updater refuses a
                // channel build that is behind this machine, which is what stopped a machine
                // ahead of its channel from being rolled back by its own update button.
                Param::flag("allow_downgrade")
                    .describe(
                        "Install the channel's build even though it is OLDER than what is \
                         installed — a deliberate rollback of this machine to the channel",
                    )
                    .optional(),
            ),
            move |args| {
                let channel = args["channel"]
                    .as_str()
                    .filter(|c| !c.trim().is_empty())
                    .map(|c| c.trim().to_string());
                let allow_downgrade = args["allow_downgrade"].as_bool() == Some(true);
                // `spawn_apply` detaches with its output thrown away, because the first thing
                // the updater does is stop this shell — so anything it refuses on the far side
                // of that is invisible here, and this action would answer "applying" about an
                // install that was never going to happen. The one refusal that can be seen
                // coming is checked first: a channel behind this machine is a downgrade, and
                // the updater will not do one that was not asked for by name.
                if !allow_downgrade {
                    if let CheckOutcome::Ahead { reason, .. } = check_now(channel.as_deref()) {
                        return Err(format!(
                            "nothing to apply — {reason}. Installing the channel's build would \
                             roll this machine back; pass allow_downgrade to do that on purpose"
                        ));
                    }
                }
                spawn_apply(
                    channel.as_deref(),
                    args["force"].as_bool() == Some(true),
                    allow_downgrade,
                )?;
                Ok(serde_json::json!({
                    "applying": true,
                    "channel": channel.unwrap_or_else(|| "configured".to_string()),
                    "downgrade": allow_downgrade,
                    "watch": "the shell will restart; reconnect and `describe shell`, or run `yantrik-update status`, to see the new build. A shell that fails to come up is rolled back automatically.",
                }))
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exactly what the script prints. Kept verbatim rather than built from a helper, so a
    // change to the script's porcelain breaks a test that shows the old text next to the new.
    const UP_TO_DATE: &str = "\
result=up_to_date
channel=nightly
channel_source=update.conf
host=releases.yantrikos.com
scheme=https
installed_version=0.4.2
installed_git=deadbee
channel_version=0.4.2
channel_git=deadbee
reason=installed build matches channel 'nightly'
";

    const AVAILABLE: &str = "\
result=update_available
channel=nightly
channel_source=BUILD
host=releases.yantrikos.com
scheme=https
installed_version=0.4.0
installed_git=0000001
channel_version=0.4.2
channel_git=deadbee
reason=0000001 → deadbee
";

    // The machine this was found on: a developer deploy from main, and a nightly channel about
    // 150 commits behind it.
    const AHEAD: &str = "\
result=ahead_of_channel
channel=nightly
channel_source=BUILD
host=releases.yantrikos.com
scheme=https
installed_version=v0.1.0-456-gca376be-dev
installed_git=ca376be
channel_version=v0.1.0-304
channel_git=7bc7d6b
reason=installed v0.1.0-456-gca376be-dev (a developer deploy) is ahead of channel 'nightly' (v0.1.0-304)
";

    const NOT_PUBLISHED: &str = "\
result=not_published
channel=stable
channel_source=default
host=releases.yantrikos.com
scheme=https
installed_version=0.4.2
installed_git=deadbee
channel_version=
channel_git=
published=nightly
reason=channel 'stable' is not published — the server has nothing on it (published: nightly)
";

    const UNREACHABLE: &str = "\
result=unreachable
channel=nightly
channel_source=update.conf
host=releases.yantrikos.com
scheme=https
installed_version=0.4.2
installed_git=deadbee
channel_version=
channel_git=
reason=cannot reach releases.yantrikos.com over https (Name or service not known)
";

    #[test]
    fn up_to_date_is_up_to_date() {
        let out = parse_check(0, UP_TO_DATE, "");
        assert_eq!(
            out,
            CheckOutcome::UpToDate {
                installed: "0.4.2".into(),
                git: "deadbee".into()
            }
        );
        assert!(!out.can_install());
        assert_eq!(out.state(), "up-to-date");
        assert!(out.headline().contains("0.4.2 (deadbee)"), "{}", out.headline());
    }

    #[test]
    fn update_available_carries_both_ends() {
        let out = parse_check(10, AVAILABLE, "");
        assert_eq!(
            out,
            CheckOutcome::UpdateAvailable {
                from: "0000001".into(),
                to: "deadbee".into(),
                reason: "0000001 → deadbee".into(),
            }
        );
        assert!(out.can_install());
        assert!(out.headline().contains("0000001 → deadbee"));
    }

    /// A channel that is BEHIND this machine is not an update, and this is the shape that used
    /// to be read as one: the same porcelain with `result=update_available` on it produced
    /// "Update available — ca376be → 7bc7d6b" and an Install button that rolled the machine back
    /// a day of fixes. The comparison is the script's; what is held here is that the answer it
    /// gives for it arrives intact, offers nothing, and does not read as a failure.
    #[test]
    fn a_channel_behind_this_machine_offers_nothing_to_install() {
        let out = parse_check(13, AHEAD, "");
        match &out {
            CheckOutcome::Ahead { installed, channel_build, channel, reason } => {
                assert_eq!(installed, "v0.1.0-456-gca376be-dev (ca376be)");
                assert_eq!(channel_build, "v0.1.0-304 (7bc7d6b)");
                assert_eq!(channel, "nightly");
                assert!(reason.contains("ahead of channel 'nightly'"), "{reason}");
            }
            other => panic!("wanted Ahead, got {other:?}"),
        }
        assert!(!out.can_install(), "a machine ahead of its channel was offered an install");
        assert_eq!(out.state(), "ahead");
        let headline = out.headline();
        assert!(headline.starts_with("Nothing to update"), "{headline}");
        assert!(!headline.contains("Could not check"), "{headline}");
    }

    /// The fallback sentence, for a script that sends the result without its own reason. It
    /// still has to say which way round the two builds are — "nothing to update" on its own is
    /// indistinguishable from a check that quietly failed.
    #[test]
    fn ahead_with_no_reason_still_names_both_builds() {
        let out = parse_check(
            13,
            "result=ahead_of_channel\nchannel=beta\ninstalled_version=v0.2.0-3-gaaa1111\n\
             installed_git=aaa1111\nchannel_version=v0.1.0-468-gbbb2222\nchannel_git=bbb2222\n",
            "",
        );
        assert_eq!(
            out.headline(),
            "Nothing to update — this machine is ahead of channel 'beta': installed \
             v0.2.0-3-gaaa1111 (aaa1111), the channel has v0.1.0-468-gbbb2222 (bbb2222)"
        );
        assert!(!out.can_install());
    }

    #[test]
    fn not_published_is_its_own_answer() {
        let out = parse_check(11, NOT_PUBLISHED, "");
        match &out {
            CheckOutcome::NotPublished { channel, reason } => {
                assert_eq!(channel, "stable");
                assert!(reason.contains("not published"), "{reason}");
                // The sentence a person can act on: it names what IS published.
                assert!(reason.contains("nightly"), "{reason}");
            }
            other => panic!("wanted NotPublished, got {other:?}"),
        }
        assert!(!out.can_install());
        assert_eq!(out.state(), "not-published");
    }

    #[test]
    fn unreachable_is_not_not_published() {
        let out = parse_check(12, UNREACHABLE, "");
        match &out {
            CheckOutcome::Unreachable { reason } => {
                assert!(reason.contains("cannot reach"), "{reason}")
            }
            other => panic!("wanted Unreachable, got {other:?}"),
        }
        assert!(!out.can_install());
    }

    /// The rule the old checker broke, stated as a test: NOTHING that is not an explicit
    /// success is ever read as one. `wire/version.rs` returned `no_update` from every error
    /// path, so a machine with no DNS was told "All components up to date".
    #[test]
    fn no_failure_shape_can_report_up_to_date() {
        let failures: Vec<(i32, &str, &str)> = vec![
            // Nothing at all — updater missing or killed.
            (-1, "", ""),
            (1, "", "FAIL: need 'python3' but it is not installed"),
            // An older updater that predates --porcelain and answers in prose.
            (0, "Checking channel 'nightly' on releases.yantrikos.com\nup to date\n", ""),
            // Prose that happens to contain the words.
            (0, "everything is result of up_to_date\n", ""),
            // A result this build does not know.
            (0, "result=probably_fine\nchannel=nightly\n", ""),
            // Porcelain that says current and an exit code that says otherwise.
            (12, UP_TO_DATE, ""),
            // Truncated output: a key with no value, no result line.
            (0, "channel=\nhost=\n", ""),
            // A server error body that happens to be key=value shaped.
            (1, "error=502 Bad Gateway\n", ""),
        ];
        for (code, stdout, stderr) in failures {
            let out = parse_check(code, stdout, stderr);
            assert!(
                !matches!(out, CheckOutcome::UpToDate { .. }),
                "exit {code} with stdout {stdout:?} was read as up-to-date: {out:?}"
            );
            assert!(
                !out.can_install(),
                "exit {code} with stdout {stdout:?} offered an install it cannot do: {out:?}"
            );
            assert!(
                out.headline().starts_with("Could not check"),
                "exit {code} did not say it could not check: {}",
                out.headline()
            );
        }
    }

    #[test]
    fn error_carries_the_scripts_own_sentence() {
        let out = parse_check(1, "", "\u{1b}[31mFAIL: need 'curl' but it is not installed\u{1b}[0m");
        match out {
            CheckOutcome::Error { reason } => {
                // The colour is stripped and the sentence survives — it is the only thing in
                // the failure a person can act on.
                assert_eq!(reason, "FAIL: need 'curl' but it is not installed");
            }
            other => panic!("wanted Error, got {other:?}"),
        }
    }

    #[test]
    fn status_parses_the_configured_channel_and_host() {
        let s = parse_status(
            "channel=nightly\nchannel_source=BUILD\nhost=releases.yantrikos.com\nscheme=https\n\
             conf=/opt/yantrik/update.conf\nconf_present=no\nconf_writable=yes\n\
             version=0.4.2\ngit=deadbee\nname=yantrik-os-0.4.2\ninstalled=2026-09-20T10:00:00Z\n\
             backups=2\n",
        );
        assert_eq!(s.channel, "nightly");
        assert_eq!(s.channel_source, "BUILD");
        assert_eq!(s.host, "releases.yantrikos.com");
        assert_eq!(s.scheme, "https");
        assert_eq!(s.installed_label(), "0.4.2 (deadbee)");
        assert!(s.conf_writable);
    }

    #[test]
    fn status_of_nothing_is_empty_not_invented() {
        let s = parse_status("");
        assert_eq!(s, UpdateStatus::default());
        assert_eq!(s.installed_label(), "unknown");
        assert!(!s.conf_writable, "an unreadable status must not claim it can be written");
    }

    #[test]
    fn kv_ignores_prose_and_comments() {
        let kv = parse_kv("# a comment\nChecking channel 'nightly' on host\nchannel=beta\n\nx=1=2\n");
        assert_eq!(kv.get("channel").map(String::as_str), Some("beta"));
        // Only the first `=` splits, so a value containing one survives whole.
        assert_eq!(kv.get("x").map(String::as_str), Some("1=2"));
        assert_eq!(kv.len(), 2, "prose lines became keys: {kv:?}");
    }

    #[test]
    fn set_channel_refuses_a_name_that_is_not_a_channel() {
        // Refused before the process is started, so a typo cannot reach the file at all.
        let err = set_channel("hourly").unwrap_err();
        assert!(err.contains("not a channel"), "{err}");
    }
}
