//! Debian package operations, for the Package Manager screen.
//!
//! The screen was written against `apk`, Alpine's package manager. This OS is Debian: `apk` is
//! not installed, `apt-get` is. So the one screen whose purpose is installing software could
//! not install software, and had not been able to since the image became Debian-based.
//!
//! Everything here shells out, because that is what a package manager is. The split is
//! deliberate: the functions that RUN things are thin, and the functions that PARSE their
//! output are pure and tested. Parsing is where this kind of code is actually wrong, and it is
//! the half that does not need a machine with 665 packages on it to check.
//!
//! Privileges: mutations go through `sudo`, which the desktop user has passwordless. That is a
//! deliberate choice recorded here rather than discovered later — apps installed this way run
//! unsandboxed, with the user's full access, exactly as on any ordinary Debian desktop.

/// One package as the list shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: String,
    pub installed: bool,
    pub upgradable: bool,
    pub size_text: String,
    pub repo: String,
}

/// Everything the detail pane shows about one package.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub name: String,
    pub version: String,
    pub description: String,
    pub maintainer: String,
    pub dependencies: String,
    pub size: String,
    pub repo: String,
    pub installed: bool,
    pub upgradable: bool,
}

// ── Running things ──────────────────────────────────────────────────────────────────────

/// Everything installed, from dpkg's own database.
///
/// `dpkg-query` rather than `apt list --installed`: it takes a format string, so the output is
/// tab-separated fields we choose instead of a human-readable line we would have to unpick.
/// `apt` also prints "WARNING: apt does not have a stable CLI interface" when its output is not
/// a terminal, which is a fair warning to take seriously.
pub fn list_installed() -> Result<Vec<Package>, String> {
    let out = std::process::Command::new("dpkg-query")
        .args([
            "-W",
            "-f=${Package}\t${Version}\t${Installed-Size}\t${Section}\t${binary:Summary}\n",
        ])
        .output()
        .map_err(|e| format!("could not run dpkg-query: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "dpkg-query failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_installed(&String::from_utf8_lossy(&out.stdout)))
}

/// Names of packages with a newer version available.
pub fn list_upgradable() -> Vec<String> {
    let out = std::process::Command::new("apt")
        .args(["list", "--upgradable"])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            parse_upgradable(&String::from_utf8_lossy(&o.stdout))
        }
        // Not fatal: a machine that has never run `apt update` simply has nothing to report.
        _ => Vec::new(),
    }
}

/// Packages matching a query, whether installed or not.
pub fn search(query: &str) -> Vec<Package> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let out = std::process::Command::new("apt-cache")
        .args(["search", "--names-only", query])
        .output();
    match out {
        Ok(o) if o.status.success() => parse_search(&String::from_utf8_lossy(&o.stdout)),
        _ => Vec::new(),
    }
}

/// Detail for one package.
pub fn detail(name: &str, installed: bool, upgradable: bool) -> Detail {
    let out = std::process::Command::new("apt-cache")
        .args(["show", name])
        .output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    };
    let mut d = parse_detail(&text);
    d.name = name.to_string();
    d.installed = installed;
    d.upgradable = upgradable;
    d
}

/// The Package Manager's privileged half (#397): the only way this account runs apt without a
/// password. It takes package NAMES only (no local .deb, no apt options) and sets the
/// noninteractive frontend itself, since apt would otherwise stop on a configuration prompt and
/// wait for a terminal that does not exist. Root-owned, outside /opt/yantrik, which its user can
/// write. See deploy/yantrik-os/yantrik-pkg.
pub const PKG_HELPER: &str = "/usr/lib/yantrik/yantrik-pkg";

fn helper(verb: &str, pkg: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = vec!["sudo".into(), "-n".into(), PKG_HELPER.into(), verb.into()];
    v.extend(pkg.map(str::to_string));
    v
}

/// The command that refreshes the package index.
pub fn update_command() -> Vec<String> {
    helper("update", None)
}

/// The command that installs `pkg`.
pub fn install_command(pkg: &str) -> Vec<String> {
    helper("install", Some(pkg))
}

/// The command that removes `pkg`.
pub fn remove_command(pkg: &str) -> Vec<String> {
    helper("remove", Some(pkg))
}

/// The command that upgrades one package without pulling in a full distribution upgrade.
pub fn upgrade_one_command(pkg: &str) -> Vec<String> {
    helper("upgrade-one", Some(pkg))
}

/// The command that upgrades everything already installed.
pub fn upgrade_all_command() -> Vec<String> {
    helper("upgrade", None)
}

// ── Parsing ─────────────────────────────────────────────────────────────────────────────

/// dpkg-query output: `name<TAB>version<TAB>installed-size-kb<TAB>section<TAB>summary`.
pub fn parse_installed(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut f = line.split('\t');
        let name = f.next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let version = f.next().unwrap_or("").trim();
        let size_kb = f.next().unwrap_or("").trim();
        let section = f.next().unwrap_or("").trim();
        let summary = f.next().unwrap_or("").trim();

        out.push(Package {
            name: name.to_string(),
            version: version.to_string(),
            description: summary.to_string(),
            installed: true,
            upgradable: false,
            size_text: human_size_kb(size_kb),
            repo: section.to_string(),
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// `apt list --upgradable` output. The first line is a "Listing..." header, and each entry
/// looks like `zlib1g/stable 1:1.3.dfsg-3 amd64 [upgradable from: 1:1.2.13.dfsg-1]`.
pub fn parse_upgradable(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Listing") || line.starts_with("WARNING") {
            continue;
        }
        if let Some((name, _)) = line.split_once('/') {
            let name = name.trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// `apt-cache search` output: `name - short description`, one per line.
pub fn parse_search(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, desc) = match line.split_once(" - ") {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (line, ""),
        };
        if name.is_empty() {
            continue;
        }
        out.push(Package {
            name: name.to_string(),
            description: desc.to_string(),
            ..Default::default()
        });
    }
    out
}

/// `apt-cache show` output — RFC822-ish `Field: value`, with continuation lines indented by a
/// space and a lone `.` meaning a blank line inside a description.
pub fn parse_detail(text: &str) -> Detail {
    let mut d = Detail::default();
    let mut desc: Vec<String> = Vec::new();
    let mut in_description = false;

    for line in text.lines() {
        // A continuation of the field above.
        if line.starts_with(' ') || line.starts_with('\t') {
            if in_description {
                let body = line.trim();
                desc.push(if body == "." { String::new() } else { body.to_string() });
            }
            continue;
        }

        in_description = false;
        let Some((field, value)) = line.split_once(':') else {
            // A blank line separates records; only the first record is this package.
            if line.trim().is_empty() && !d.name.is_empty() {
                break;
            }
            continue;
        };
        let value = value.trim();

        match field.trim().to_ascii_lowercase().as_str() {
            "package" => d.name = value.to_string(),
            "version" if d.version.is_empty() => d.version = value.to_string(),
            "maintainer" => d.maintainer = value.to_string(),
            "section" => d.repo = value.to_string(),
            "installed-size" => d.size = human_size_kb(value),
            "depends" => d.dependencies = value.to_string(),
            "description" | "description-en" => {
                if !value.is_empty() {
                    desc.push(value.to_string());
                }
                in_description = true;
            }
            _ => {}
        }
    }

    d.description = desc.join("\n").trim().to_string();
    d
}

/// dpkg reports installed size in kibibytes, with no unit.
fn human_size_kb(kb: &str) -> String {
    let Ok(kb) = kb.trim().parse::<f64>() else {
        return String::new();
    };
    if kb <= 0.0 {
        return String::new();
    }
    if kb < 1024.0 {
        format!("{kb:.0} KB")
    } else if kb < 1024.0 * 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{:.1} GB", kb / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_packages_come_back_sorted_with_their_summary() {
        let text = "zlib1g\t1:1.3.dfsg-3\t168\tlibs\tcompression library - runtime\n\
                    bash\t5.2.15-2\t7000\tshells\tGNU Bourne Again SHell\n";
        let pkgs = parse_installed(text);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "bash", "sorted by name, case-insensitively");
        assert_eq!(pkgs[0].version, "5.2.15-2");
        assert_eq!(pkgs[0].description, "GNU Bourne Again SHell");
        assert_eq!(pkgs[0].size_text, "6.8 MB");
        assert_eq!(pkgs[0].repo, "shells");
        assert!(pkgs[0].installed);
        assert_eq!(pkgs[1].size_text, "168 KB");
    }

    #[test]
    fn a_summary_containing_a_dash_survives() {
        // The apk parser split descriptions on " - ", which would have cut this one in half.
        let pkgs = parse_installed("zlib1g\t1:1.3\t168\tlibs\tcompression library - runtime\n");
        assert_eq!(pkgs[0].description, "compression library - runtime");
    }

    #[test]
    fn the_listing_header_is_not_a_package() {
        let text = "Listing...\n\
                    zlib1g/stable 1:1.3.dfsg-3 amd64 [upgradable from: 1:1.2.13.dfsg-1]\n\
                    bash/stable 5.2.15-3 amd64 [upgradable from: 5.2.15-2]\n";
        assert_eq!(parse_upgradable(text), vec!["zlib1g", "bash"]);
    }

    #[test]
    fn apt_warning_lines_are_not_packages() {
        // apt prints this to stdout whenever it is not talking to a terminal, which is always
        // here. Treating it as a package name would put it in the list.
        let text = "WARNING: apt does not have a stable CLI interface.\nListing...\nvim/stable 2:9.0 amd64 [upgradable from: 2:8.2]\n";
        assert_eq!(parse_upgradable(text), vec!["vim"]);
    }

    #[test]
    fn search_splits_name_from_description_once() {
        let text = "ripgrep - fast line-oriented search tool - recursive\nfd-find - simple find\n";
        let hits = parse_search(text);
        assert_eq!(hits[0].name, "ripgrep");
        assert_eq!(hits[0].description, "fast line-oriented search tool - recursive");
        assert_eq!(hits[1].name, "fd-find");
    }

    #[test]
    fn detail_reads_the_multi_line_description() {
        let text = "Package: nano\n\
                    Version: 7.2-1\n\
                    Installed-Size: 1024\n\
                    Maintainer: Jordi Mallach <jordi@debian.org>\n\
                    Depends: libc6, libncursesw6\n\
                    Section: editors\n\
                    Description-en: small, friendly text editor\n\
                    \x20GNU nano is an easy-to-use text editor.\n\
                    \x20.\n\
                    \x20It is a free replacement for Pico.\n";
        let d = parse_detail(text);
        assert_eq!(d.name, "nano");
        assert_eq!(d.version, "7.2-1");
        assert_eq!(d.size, "1.0 MB");
        assert_eq!(d.maintainer, "Jordi Mallach <jordi@debian.org>");
        assert_eq!(d.dependencies, "libc6, libncursesw6");
        assert_eq!(d.repo, "editors");
        assert!(d.description.starts_with("small, friendly text editor"));
        assert!(d.description.contains("free replacement for Pico"));
        assert!(d.description.contains("\n\n"), "a lone dot is a blank line, not a full stop");
    }

    #[test]
    fn detail_stops_at_the_first_record() {
        // `apt-cache show` prints every available version. The pane describes one package, so
        // the second record must not overwrite the first.
        let text = "Package: vim\nVersion: 2:9.0-1\n\nPackage: vim\nVersion: 2:8.2-1\n";
        assert_eq!(parse_detail(text).version, "2:9.0-1");
    }

    #[test]
    fn mutations_go_through_the_package_helper_and_never_apt_directly() {
        // Passwordless `apt-get install <anything>` was root for anything that reached it (#397):
        // every mutation is the helper, by its root-owned path, with sudo that never prompts.
        let cmd = install_command("ripgrep");
        assert_eq!(cmd, ["sudo", "-n", PKG_HELPER, "install", "ripgrep"]);
        assert_eq!(update_command(), ["sudo", "-n", PKG_HELPER, "update"]);
        assert_eq!(upgrade_all_command(), ["sudo", "-n", PKG_HELPER, "upgrade"]);
        assert_eq!(upgrade_one_command("vim"), ["sudo", "-n", PKG_HELPER, "upgrade-one", "vim"]);
        assert_eq!(remove_command("vim"), ["sudo", "-n", PKG_HELPER, "remove", "vim"]);
        assert!(!PKG_HELPER.starts_with("/opt/yantrik"), "never where its user can write");
    }

    #[test]
    fn a_size_that_is_not_a_number_is_simply_absent() {
        assert_eq!(human_size_kb(""), "");
        assert_eq!(human_size_kb("unknown"), "");
        assert_eq!(human_size_kb("0"), "");
    }
}
