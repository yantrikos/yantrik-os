//! Subordinate ids for the account the installer creates (#401).
//!
//! Rootless podman, which the Container Manager runs, maps a container's users onto ids the
//! account owns in /etc/subuid and /etc/subgid. With none it can run only images that use a single
//! user, and most pulls fail with "potentially insufficient UIDs or GIDs". Debian's useradd hands a
//! range out when /etc/subuid exists, and the image makes sure it does; this checks rather than
//! assumes, and adds the range useradd would have given when it is missing. `yantrik-update
//! reconcile` holds the same rule for machines installed before this was here.

use super::chroot_cmd;

/// How many ids one account gets: every uid a container's own /etc/passwd can name.
pub const COUNT: u64 = 65_536;

/// Where ranges start: above every uid Debian gives an account or a system user.
pub const FLOOR: u64 = 100_000;

/// The inclusive range to add for `user` (uid `uid`), given the contents of /etc/subuid or
/// /etc/subgid, or `None` when the account already owns a full range.
///
/// An entry may name the account or its uid. A new range starts past the end of every range
/// already handed out: two accounts sharing ids could reach into each other's containers.
pub fn range_to_add(contents: &str, user: &str, uid: u32) -> Option<(u64, u64)> {
    let uid = uid.to_string();
    let mut owned = 0u64;
    let mut top = FLOOR;
    for line in contents.lines() {
        let mut fields = line.trim().split(':');
        let (Some(who), Some(start), Some(count)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Ok(start), Ok(count)) = (start.parse::<u64>(), count.parse::<u64>()) else {
            continue;
        };
        if who == user || who == uid {
            owned = owned.saturating_add(count);
        }
        top = top.max(start.saturating_add(count));
    }
    (owned < COUNT).then(|| (top, top + COUNT - 1))
}

/// Give `username` a subordinate uid and gid range in the installed system, if it has none.
/// Never fails the install: a machine without one still works, and its first update adds it.
pub fn ensure(mount_dir: &str, username: &str) {
    let uid = match chroot_cmd(mount_dir, &["id", "-u", username]) {
        Ok(out) => match out.trim().parse::<u32>() {
            Ok(uid) => uid,
            Err(_) => {
                tracing::warn!(user = username, output = %out.trim(), "installer: no uid to give subordinate ids to");
                return;
            }
        },
        Err(e) => {
            tracing::warn!(user = username, error = %e, "installer: no uid to give subordinate ids to");
            return;
        }
    };
    for (file, flag) in [("subuid", "--add-subuids"), ("subgid", "--add-subgids")] {
        let path = format!("{mount_dir}/etc/{file}");
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        let Some((first, last)) = range_to_add(&contents, username, uid) else {
            continue;
        };
        // usermod will not add a range to a file that does not exist.
        if !std::path::Path::new(&path).exists() {
            let _ = chroot_cmd(
                mount_dir,
                &[
                    "install",
                    "-m",
                    "0644",
                    "/dev/null",
                    &format!("/etc/{file}"),
                ],
            );
        }
        let range = format!("{first}-{last}");
        match chroot_cmd(mount_dir, &["usermod", flag, &range, username]) {
            Ok(_) => {
                tracing::info!(user = username, file, range = %range, "installer: subordinate ids added")
            }
            Err(e) => {
                tracing::warn!(user = username, file, error = %e, "installer: could not add subordinate ids; the first update will")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_missing_file_starts_at_the_floor() {
        assert_eq!(range_to_add("", "pranab", 1000), Some((100_000, 165_535)));
    }

    #[test]
    fn an_account_with_a_full_range_gets_nothing() {
        assert_eq!(range_to_add("pranab:100000:65536\n", "pranab", 1000), None);
        // Named by uid, as some tools write it.
        assert_eq!(range_to_add("1000:100000:65536\n", "pranab", 1000), None);
    }

    #[test]
    fn a_new_range_starts_past_every_range_handed_out() {
        let file = "yantrik:100000:65536\n# a note\nbuild:300000:65536\n";
        assert_eq!(range_to_add(file, "pranab", 1001), Some((365_536, 431_071)));
    }

    #[test]
    fn short_ranges_that_sum_under_a_full_one_get_a_full_one_more() {
        let file = "pranab:100000:1000\npranab:200000:1000\n";
        assert_eq!(range_to_add(file, "pranab", 1000), Some((201_000, 266_535)));
    }

    #[test]
    fn lines_that_are_not_ranges_are_skipped() {
        let file = "garbage\npranab:x:65536\n:::\n";
        assert_eq!(range_to_add(file, "pranab", 1000), Some((100_000, 165_535)));
    }
}
