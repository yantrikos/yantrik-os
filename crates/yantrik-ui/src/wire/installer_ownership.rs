//! Whose files the installed system's /opt/yantrik is (#397): root's, except what the desktop's
//! account keeps (logs/, data/ and config.yaml).
//!
//! The installer copies the live system, and on the live system the tree is the live user's and
//! `/etc/sudoers.d/yantrik` lets that user run anything as root without a password: the live
//! session runs this installer and needs root without a terminal. Neither may reach an installed
//! machine. `yantrik-update migrate-ownership` is the one place that turns the copied tree into
//! the installed layout and swaps the blanket rule for its narrow one, so it is what runs first.
//!
//! When it fails, the installer used to log a warning and carry on, and the machine kept both the
//! blanket rule and a tree its user can write: anything running as the person (an agent harness,
//! a mind with a terminal) was root without asking. Security review of #614, 4 October 2026: that
//! failed open. Now the installer locks the tree down itself, removes the blanket rule, and tells
//! the person; and if even that fails, the install fails rather than finish an unsafe machine.

use super::installer::chroot_cmd;

/// The tree, inside the target root.
const PREFIX: &str = "/opt/yantrik";
/// The updater that owns the installed layout and its sudo rule.
const UPDATER: &str = "/opt/yantrik/bin/yantrik-update";
/// The live image's blanket rule (build-debian-iso.sh), copied onto the target with the system.
const BLANKET_RULE: &str = "/etc/sudoers.d/yantrik";

/// What the person reads when the updater could not secure the tree and the installer did.
/// The machine is safe either way; what it lacks is the narrow rule that lets updates run
/// without a password, and one command with the person's password puts it in place.
pub(super) const LOCKED_DOWN_NOTE: &str = "Yantrik could not finish securing its system files, \
so the installer locked them down itself. If updates ask for your password, run \
`sudo yantrik-update migrate-ownership` once in a terminal.";

/// Make the target's tree root's. `Ok(None)`: the updater did it, as it always should.
/// `Ok(Some(note))`: the updater failed, the installer locked the tree down, and `note` is for
/// the person. `Err`: neither worked, and the install must not finish.
pub(super) fn secure_target(mount_dir: &str, owner: &str) -> Result<Option<String>, String> {
    let migrate_error = match chroot_cmd(mount_dir, &[UPDATER, "migrate-ownership"]) {
        Ok(_) => return Ok(None),
        Err(e) => e,
    };
    tracing::error!(error = %migrate_error, "yantrik-update migrate-ownership failed on the target; locking /opt/yantrik down");

    let has_config = std::path::Path::new(&format!("{mount_dir}{PREFIX}/config.yaml")).exists();
    for command in lockdown_commands(owner, has_config) {
        let args: Vec<&str> = command.iter().map(String::as_str).collect();
        chroot_cmd(mount_dir, &args).map_err(|e| {
            format!(
                "could not make the installed system's files root's (yantrik-update: {}; then {}: {e})",
                migrate_error.trim(),
                args.join(" ")
            )
        })?;
    }
    tracing::warn!("/opt/yantrik locked down by the installer; the narrow sudo rule waits for migrate-ownership");
    Ok(Some(LOCKED_DOWN_NOTE.to_string()))
}

/// The commands, run in the target root, that leave the tree fail-closed without the updater:
/// the blanket rule gone, everything under /opt/yantrik root's and written by root alone, and
/// then the desktop's account given back what it keeps, private to it, as migrate-ownership
/// leaves them. Stricter than migrate-ownership on anything it does not name, which is the point
/// of a fallback. No narrow rule is written here: the updater is that rule's one author.
///
/// Root first and the hand-back after, and `-R` never follows a link, so a link in logs/ ends up
/// the owner's link and nothing outside the tree is touched.
pub(super) fn lockdown_commands(owner: &str, has_config: bool) -> Vec<Vec<String>> {
    let own = format!("{owner}:{owner}");
    let logs = format!("{PREFIX}/logs");
    let data = format!("{PREFIX}/data");
    let config = format!("{PREFIX}/config.yaml");
    let mut commands: Vec<Vec<&str>> = vec![
        vec!["rm", "-f", BLANKET_RULE],
        vec!["chown", "-R", "root:root", PREFIX],
        vec!["chmod", "-R", "go-w", PREFIX],
        vec!["mkdir", "-p", &logs, &data],
        vec!["chown", "-R", &own, &logs, &data],
        vec!["chmod", "go-rwx", &logs, &data],
    ];
    if has_config {
        // The person's provider settings: Settings and onboarding write it as them, and it is
        // read by its owner alone (private_config in yantrik-update).
        commands.push(vec!["chown", &own, &config]);
        commands.push(vec!["chmod", "0600", &config]);
    }
    commands
        .into_iter()
        .map(|c| c.into_iter().map(String::from).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(commands: &[Vec<String>], wanted: &[&str]) -> Option<usize> {
        commands.iter().position(|c| c == wanted)
    }

    #[test]
    fn the_blanket_rule_goes_first() {
        let commands = lockdown_commands("asha", true);
        assert_eq!(commands[0], ["rm", "-f", "/etc/sudoers.d/yantrik"]);
    }

    #[test]
    fn the_whole_tree_becomes_roots_and_only_root_writes_it() {
        let commands = lockdown_commands("asha", true);
        assert!(position(&commands, &["chown", "-R", "root:root", "/opt/yantrik"]).is_some());
        assert!(position(&commands, &["chmod", "-R", "go-w", "/opt/yantrik"]).is_some());
    }

    #[test]
    fn the_person_gets_back_logs_data_and_config_after_root_takes_the_rest() {
        let commands = lockdown_commands("asha", true);
        let root = position(&commands, &["chown", "-R", "root:root", "/opt/yantrik"]).unwrap();
        let back = position(
            &commands,
            &["chown", "-R", "asha:asha", "/opt/yantrik/logs", "/opt/yantrik/data"],
        )
        .expect("logs and data go back to the person");
        assert!(root < back, "the hand-back must come after root takes the tree");
        let config = position(&commands, &["chown", "asha:asha", "/opt/yantrik/config.yaml"])
            .expect("config.yaml goes back to the person");
        assert!(root < config);
        assert!(position(&commands, &["chmod", "0600", "/opt/yantrik/config.yaml"]).is_some());
    }

    #[test]
    fn nothing_but_logs_data_and_config_is_handed_back() {
        let kept = ["/opt/yantrik/logs", "/opt/yantrik/data", "/opt/yantrik/config.yaml"];
        for command in lockdown_commands("asha", true) {
            if command.iter().any(|a| a.contains("asha")) {
                for path in command.iter().filter(|a| a.starts_with('/')) {
                    assert!(kept.contains(&path.as_str()), "{path} must stay root's");
                }
            }
        }
    }

    #[test]
    fn a_missing_config_is_not_a_failed_install() {
        let commands = lockdown_commands("asha", false);
        assert!(commands.iter().all(|c| !c.iter().any(|a| a.ends_with("config.yaml"))));
    }

    #[test]
    fn no_sudo_rule_is_written_by_the_fallback() {
        for command in lockdown_commands("asha", true) {
            assert!(
                !command.iter().any(|a| a.contains("sudoers") && a != BLANKET_RULE),
                "only yantrik-update writes a sudo rule: {command:?}"
            );
        }
    }
}
