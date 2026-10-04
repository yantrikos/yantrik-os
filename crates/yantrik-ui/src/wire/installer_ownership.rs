//! Whose files the installed system's /opt/yantrik is (#397): root's, except what the desktop's
//! account keeps (logs/, data/ and config.yaml).
//!
//! The installer copies the live system, and on the live system the tree is the live user's and
//! `/etc/sudoers.d/yantrik` lets that user run anything as root without a password: the live
//! session runs this installer and needs root without a terminal. Neither may reach an installed
//! machine. `yantrik-update migrate-ownership` is the one place that turns the copied tree into
//! the installed layout and writes the narrow sudo rule, so it is what runs first.
//!
//! When it failed, the installer used to log a warning and carry on, and the machine kept both the
//! blanket rule and a tree its user can write: anything running as the person (an agent harness,
//! a mind with a terminal) was root without asking. Security review of #614, 4 October 2026: that
//! failed open. And its exit code only says the sudo rule was written: its chowns are not checked,
//! and it removes the blanket rule only on an exact match (review of #616, the same day). So the
//! result is checked here rather than trusted; when it is not right, the installer locks the tree
//! down itself and tells the person; and if even that fails, the install fails.
//!
//! The text installer and cloud-init's first boot do the same through
//! deploy/yantrik-os/yantrik-lockdown; keep the two in step.

use super::installer::chroot_cmd;

/// The tree, inside the target root.
const PREFIX: &str = "/opt/yantrik";
const LOGS: &str = "/opt/yantrik/logs";
const DATA: &str = "/opt/yantrik/data";
const CONFIG: &str = "/opt/yantrik/config.yaml";
/// The updater that owns the installed layout and its sudo rule.
const UPDATER: &str = "/opt/yantrik/bin/yantrik-update";
/// The live image's blanket rule (build-debian-iso.sh), copied onto the target with the system.
const BLANKET_RULE: &str = "/etc/sudoers.d/yantrik";
/// The updater's narrow rule (SUDO_RULE in yantrik-update). Removed only when the tree cannot be
/// made root's: a password-free rule on files the person can write is root for the person.
const NARROW_RULE: &str = "/etc/sudoers.d/yantrik-os";

/// What the person reads when the updater could not secure the tree and the installer did.
/// The machine is safe either way; what it lacks is the narrow rule that lets updates run
/// without a password, and one command with the person's password puts it in place.
pub(super) const LOCKED_DOWN_NOTE: &str = "Yantrik could not finish securing its system files, \
so the installer locked them down itself. If updates ask for your password, run \
`sudo yantrik-update migrate-ownership` once in a terminal.";

/// What the hand-back needs to know about the target, read without following links.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Found {
    /// data/ is a real directory, not a link or a file.
    pub data_is_dir: bool,
    /// config.yaml is a regular file, not a link.
    pub config_is_file: bool,
}

impl Found {
    fn read(mount_dir: &str) -> Self {
        let meta = |p: &str| std::fs::symlink_metadata(format!("{mount_dir}{p}")).ok();
        Found {
            data_is_dir: meta(DATA).is_some_and(|m| m.is_dir()),
            config_is_file: meta(CONFIG).is_some_and(|m| m.is_file()),
        }
    }
}

/// Make the target's tree root's. `Ok(None)`: the updater did it (the installer may have
/// tidied what it left open). `Ok(Some(note))`: the updater failed, the installer locked the
/// tree down, and `note` is for the person. `Err`: the tree could not be made root's, and the
/// install must not finish.
pub(super) fn secure_target(mount_dir: &str, owner: &str) -> Result<Option<String>, String> {
    let migrated = run_all(mount_dir, &prepare_commands(owner))
        .and_then(|()| chroot_cmd(mount_dir, &[UPDATER, "migrate-ownership"]).map(|_| ()));

    // Whatever the updater said, the live image's blanket rule never stays.
    chroot_cmd(mount_dir, &["rm", "-f", BLANKET_RULE]).map_err(|e| refuse(mount_dir, e))?;

    if migrated.is_ok() {
        match strays(mount_dir) {
            Ok(found) if found.is_empty() => return Ok(None),
            Ok(found) => tracing::warn!(left_open = %found, "the updater left part of /opt/yantrik open; locking it down"),
            Err(e) => tracing::warn!(error = %e, "could not check /opt/yantrik after the updater; locking it down"),
        }
    } else if let Err(e) = &migrated {
        tracing::error!(error = %e, "yantrik-update migrate-ownership failed on the target; locking /opt/yantrik down");
    }

    run_all(mount_dir, &lockdown_commands(owner, Found::read(mount_dir)))
        .map_err(|e| refuse(mount_dir, e))?;
    match strays(mount_dir) {
        Ok(found) if found.is_empty() => {}
        Ok(found) => return Err(refuse(mount_dir, format!("still not root's: {found}"))),
        Err(e) => return Err(refuse(mount_dir, e)),
    }

    match migrated {
        Ok(()) => Ok(None),
        Err(_) => {
            tracing::warn!("/opt/yantrik locked down by the installer; the narrow sudo rule waits for migrate-ownership");
            Ok(Some(LOCKED_DOWN_NOTE.to_string()))
        }
    }
}

/// The tree could not be made root's: no password-free rule is left behind, and the install
/// fails with why.
fn refuse(mount_dir: &str, why: String) -> String {
    let _ = chroot_cmd(mount_dir, &["rm", "-f", NARROW_RULE]);
    format!("could not make the installed system's files root's: {}", why.trim())
}

fn run_all(mount_dir: &str, commands: &[Vec<String>]) -> Result<(), String> {
    for command in commands {
        let args: Vec<&str> = command.iter().map(String::as_str).collect();
        chroot_cmd(mount_dir, &args).map_err(|e| format!("{}: {e}", args.join(" ")))?;
    }
    Ok(())
}

/// Anything in the tree that is not root's or that another account may write, apart from what
/// the person keeps, one path per line; empty when the layout is right.
fn strays(mount_dir: &str) -> Result<String, String> {
    let command = stray_check_command();
    let args: Vec<&str> = command.iter().map(String::as_str).collect();
    chroot_cmd(mount_dir, &args).map(|out| out.trim().to_string())
}

fn owned(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// Before the updater: logs/ emptied (the live session's logs are not the person's) and made the
/// person's. The updater learns whose desktop this is from who owns logs/, and the copied one is
/// the live user's (uid 1000), which is not the person's when the install renamed the account.
pub(super) fn prepare_commands(owner: &str) -> Vec<Vec<String>> {
    vec![
        owned(&["rm", "-rf", LOGS]),
        owned(&["install", "-d", "-o", owner, "-g", owner, "-m", "0700", LOGS]),
    ]
}

/// The check that the layout is right, as yantrik-lockdown's `strays` makes it: logs/ and data/
/// pruned, config.yaml skipped, and everything else must be root's and not writable by others.
/// Links are skipped for the mode test: theirs is always 0777.
pub(super) fn stray_check_command() -> Vec<String> {
    owned(&[
        "find", PREFIX, "(", "-path", LOGS, "-o", "-path", DATA, ")", "-prune", "-o", "!", "-path",
        CONFIG, "(", "!", "-user", "root", "-o", "(", "!", "-type", "l", "-perm", "/022", ")", ")",
        "-print",
    ])
}

/// The commands, run in the target root, that leave the tree fail-closed without the updater:
/// the blanket rule gone, everything under /opt/yantrik root's and written by root alone, and
/// then the desktop's account given back what it keeps, private to it. Stricter than
/// migrate-ownership on anything it does not name, which is the point of a fallback. No sudo
/// rule is written here: the updater is that rule's one author.
///
/// `-R` never follows a link; logs/ is made anew, and data/ and config.yaml are handed back only
/// as a real directory and a regular file (`found`), so nothing outside the tree is touched and
/// nothing already in logs/ is adopted.
pub(super) fn lockdown_commands(owner: &str, found: Found) -> Vec<Vec<String>> {
    let own = format!("{owner}:{owner}");
    let mut commands = vec![
        owned(&["rm", "-f", BLANKET_RULE]),
        owned(&["chown", "-R", "root:root", PREFIX]),
        owned(&["chmod", "-R", "go-w", PREFIX]),
        owned(&["rm", "-rf", LOGS]),
    ];
    if !found.data_is_dir {
        commands.push(owned(&["rm", "-f", DATA]));
    }
    commands.push(owned(&["install", "-d", "-o", owner, "-g", owner, "-m", "0700", LOGS, DATA]));
    commands.push(owned(&["chown", "-R", &own, DATA]));
    commands.push(owned(&["chmod", "0700", DATA]));
    if found.config_is_file {
        // The person's provider settings: Settings and onboarding write it as them, and it is
        // read by its owner alone (private_config in yantrik-update).
        commands.push(owned(&["chown", "-h", &own, CONFIG]));
        commands.push(owned(&["chmod", "0600", CONFIG]));
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_THERE: Found = Found { data_is_dir: true, config_is_file: true };

    fn position(commands: &[Vec<String>], wanted: &[&str]) -> Option<usize> {
        commands.iter().position(|c| c == wanted)
    }

    #[test]
    fn logs_are_emptied_and_the_persons_before_the_updater_runs() {
        let commands = prepare_commands("asha");
        assert_eq!(commands[0], ["rm", "-rf", "/opt/yantrik/logs"]);
        assert_eq!(
            commands[1],
            ["install", "-d", "-o", "asha", "-g", "asha", "-m", "0700", "/opt/yantrik/logs"]
        );
    }

    #[test]
    fn the_check_skips_only_what_the_person_keeps() {
        let check = stray_check_command().join(" ");
        assert!(check.starts_with("find /opt/yantrik "));
        assert!(check.contains("( -path /opt/yantrik/logs -o -path /opt/yantrik/data ) -prune"));
        assert!(check.contains("! -path /opt/yantrik/config.yaml"));
        assert!(check.contains("! -user root"));
        assert!(check.contains("-perm /022"));
        assert!(check.ends_with("-print"));
    }

    #[test]
    fn the_blanket_rule_goes_first() {
        let commands = lockdown_commands("asha", ALL_THERE);
        assert_eq!(commands[0], ["rm", "-f", "/etc/sudoers.d/yantrik"]);
    }

    #[test]
    fn the_whole_tree_becomes_roots_and_only_root_writes_it() {
        let commands = lockdown_commands("asha", ALL_THERE);
        assert!(position(&commands, &["chown", "-R", "root:root", "/opt/yantrik"]).is_some());
        assert!(position(&commands, &["chmod", "-R", "go-w", "/opt/yantrik"]).is_some());
    }

    #[test]
    fn the_person_gets_back_logs_data_and_config_after_root_takes_the_rest() {
        let commands = lockdown_commands("asha", ALL_THERE);
        let root = position(&commands, &["chown", "-R", "root:root", "/opt/yantrik"]).unwrap();
        let emptied = position(&commands, &["rm", "-rf", "/opt/yantrik/logs"]).unwrap();
        let made = position(
            &commands,
            &["install", "-d", "-o", "asha", "-g", "asha", "-m", "0700", "/opt/yantrik/logs", "/opt/yantrik/data"],
        )
        .expect("logs and data made the person's, private");
        let data = position(&commands, &["chown", "-R", "asha:asha", "/opt/yantrik/data"]).unwrap();
        let config = position(&commands, &["chown", "-h", "asha:asha", "/opt/yantrik/config.yaml"])
            .expect("config.yaml goes back to the person, the file itself and never a link's target");
        assert!(root < emptied && emptied < made && made < data && root < config);
        assert!(position(&commands, &["chmod", "0600", "/opt/yantrik/config.yaml"]).is_some());
    }

    #[test]
    fn nothing_but_logs_data_and_config_is_handed_back() {
        let kept = ["/opt/yantrik/logs", "/opt/yantrik/data", "/opt/yantrik/config.yaml"];
        for command in lockdown_commands("asha", ALL_THERE) {
            if command.iter().any(|a| a.contains("asha")) {
                for path in command.iter().filter(|a| a.starts_with('/')) {
                    assert!(kept.contains(&path.as_str()), "{path} must stay root's");
                }
            }
        }
    }

    #[test]
    fn a_config_that_is_missing_or_a_link_is_not_touched() {
        let commands = lockdown_commands("asha", Found { data_is_dir: true, config_is_file: false });
        assert!(commands.iter().all(|c| !c.iter().any(|a| a.ends_with("config.yaml"))));
    }

    #[test]
    fn a_data_that_is_a_link_is_removed_before_a_real_one_is_made() {
        let commands = lockdown_commands("asha", Found { data_is_dir: false, config_is_file: true });
        let removed = position(&commands, &["rm", "-f", "/opt/yantrik/data"]).expect("the link goes");
        let made = commands.iter().position(|c| c[0] == "install").unwrap();
        assert!(removed < made);
        assert!(position(&lockdown_commands("asha", ALL_THERE), &["rm", "-f", "/opt/yantrik/data"]).is_none());
    }

    #[test]
    fn no_sudo_rule_is_written_by_the_fallback() {
        for command in lockdown_commands("asha", ALL_THERE) {
            assert!(
                !command.iter().any(|a| a.contains("sudoers") && a != BLANKET_RULE),
                "only yantrik-update writes a sudo rule: {command:?}"
            );
        }
    }
}
