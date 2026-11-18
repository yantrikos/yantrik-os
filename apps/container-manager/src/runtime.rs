//! The container runtime's answers, and what they mean.
//!
//! The parts that decide what a command's result *means*, tested on a machine with no docker on
//! it (`runtime_tests.rs`). Nothing here starts a process, touches Slint or knows about the
//! window: running the runtime is `machine.rs`'s, in a job.
//!
//! What it was written to fix: every mutation in this app ran
//! `let _ = Command::new(runtime_cmd())...output();`, so a `docker rm` the daemon refused was
//! indistinguishable from one it performed, and the control surface answered `{"removed": name}`
//! either way. And `docker ps` failing produced the same empty list as `docker ps` succeeding
//! with nothing to show, so a mind reading this app on a machine without docker concluded the
//! machine was simply empty.

// ── What a command did ───────────────────────────────────────────────

/// The result of running the runtime, kept as data rather than collapsed at the call site.
///
/// "The binary is not here", "the binary is here and refused" and "the binary is here and did it"
/// are three different things to tell a person, and the code this replaces told them all the
/// same thing, which was nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// The runtime binary is not on this machine at all.
    Missing,
    /// It is on this machine and would not start — a mode bit, a broken PATH entry.
    Unstartable(String),
    /// It ran to completion. `code` is `None` when a signal ended it.
    Ran {
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

/// Stdout on success; on failure, the runtime's own words.
///
/// "Error response from daemon: No such container: web" is the only accurate description of what
/// went wrong, and the runtime has already written it. An action that failed hands that back
/// rather than a sentence of ours about what we assume happened.
pub fn outcome(command: &str, argv: &[&str], exit: Exit) -> Result<String, String> {
    match exit {
        Exit::Missing => Err(format!("{command} is not installed on this machine")),
        Exit::Unstartable(why) => Err(format!("{command} could not be started: {why}")),
        Exit::Ran {
            code: Some(0),
            stdout,
            ..
        } => Ok(stdout),
        Exit::Ran {
            code,
            stdout,
            stderr,
        } => {
            let said = first_line(&stderr).or_else(|| first_line(&stdout));
            Err(match (said, code) {
                (Some(line), _) => line,
                (None, Some(code)) => {
                    format!("`{command} {}` exited with status {code}", argv.join(" "))
                }
                (None, None) => format!("`{command} {}` was killed", argv.join(" ")),
            })
        }
    }
}

/// The first line the runtime wrote that carries anything.
///
/// Its errors are one line; a usage banner is many, and the first is still the one worth showing.
fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

// ── Whether this machine can be asked at all ─────────────────────────

/// The three states the old listing collapsed into `vec![]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// The runtime answered. An empty list now means there are no containers.
    Ready,
    /// There is no such binary on this machine.
    Missing,
    /// The binary is here but could not answer — usually a daemon that is not running, or a
    /// socket this user may not open.
    Unreachable(String),
}

/// What a listing attempt says about this machine.
pub fn availability(exit: &Exit) -> Availability {
    match exit {
        Exit::Ran { code: Some(0), .. } => Availability::Ready,
        Exit::Missing => Availability::Missing,
        Exit::Unstartable(why) => Availability::Unreachable(why.clone()),
        Exit::Ran { stdout, stderr, .. } => Availability::Unreachable(
            first_line(stderr)
                .or_else(|| first_line(stdout))
                .unwrap_or_else(|| "it would not answer".to_string()),
        ),
    }
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Availability::Ready)
    }

    /// The word `describe` publishes, so a caller can branch on it without reading English.
    pub fn state_name(&self) -> &'static str {
        match self {
            Availability::Ready => "ready",
            Availability::Missing => "not_installed",
            Availability::Unreachable(_) => "unreachable",
        }
    }

    /// The one line a person on screen and a mind reading `describe` are both owed.
    ///
    /// `None` when there is nothing wrong — an empty machine with a working runtime is not a
    /// fault, and saying so would be the mirror image of the bug this replaces.
    pub fn trouble(&self, command: &str) -> Option<String> {
        match self {
            Availability::Ready => None,
            Availability::Missing => {
                Some(format!("{command} is not installed on this machine"))
            }
            Availability::Unreachable(reason) => Some(format!(
                "the {command} daemon is not reachable: {reason}"
            )),
        }
    }
}

// ── Reading the runtime's listings ───────────────────────────────────

/// The `ps` template. One place, so the parser below and the command cannot drift apart.
pub const PS_FORMAT: &str =
    "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}\t{{.Ports}}\t{{.CreatedAt}}";

pub const IMAGES_FORMAT: &str = "{{.ID}}\t{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}";

pub const VOLUMES_FORMAT: &str = "{{.Name}}\t{{.Driver}}\t{{.Mountpoint}}";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// "running", "exited", "paused", "created", "restarting".
    pub state: String,
    pub status_text: String,
    pub ports: String,
    pub created: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Image {
    pub id: String,
    pub repo_tag: String,
    pub size_text: String,
    pub created: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    pub driver: String,
    pub mount_point: String,
}

/// Split one `--format` line into at most `fields` columns.
///
/// `splitn` rather than `split`: the last column of a `ps` line is a date that contains no tabs
/// but everything before it might, and a port mapping list is one column containing commas.
fn columns(line: &str, fields: usize) -> Vec<&str> {
    line.trim_end_matches('\r').splitn(fields, '\t').collect()
}

fn column(parts: &[&str], index: usize) -> String {
    parts.get(index).copied().unwrap_or("").trim().to_string()
}

pub fn parse_containers(text: &str) -> Vec<Container> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts = columns(line, 7);
            let state = column(&parts, 3);
            Container {
                id: column(&parts, 0),
                name: column(&parts, 1),
                image: column(&parts, 2),
                // A row whose State column did not arrive is reported stopped rather than as a
                // blank badge: the screen colours anything that is not "running" as stopped, and
                // an empty string would have read as a fourth, silent state.
                state: if state.is_empty() {
                    "stopped".to_string()
                } else {
                    state
                },
                status_text: column(&parts, 4),
                ports: column(&parts, 5),
                created: column(&parts, 6),
            }
        })
        .collect()
}

pub fn parse_images(text: &str) -> Vec<Image> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts = columns(line, 4);
            Image {
                id: column(&parts, 0),
                repo_tag: column(&parts, 1),
                size_text: column(&parts, 2),
                created: column(&parts, 3),
            }
        })
        .collect()
}

/// An image's plain first line, from its reference: `docker.io/library/alpine:latest` → "alpine,
/// the latest version"; `ghcr.io/acme/web:1.4` → "web, version 1.4". The reference stays on the
/// row as its name (it is what `docker run` takes); this is the line read first, because the sweep
/// of 17 October found rows that opened with a registry path.
pub fn plain_image_name(repo_tag: &str) -> String {
    let reference = repo_tag.trim();
    if reference.is_empty() || reference.starts_with("<none>") {
        return "An image with no name".to_string();
    }
    let (path, tag) = match reference.rsplit_once(':') {
        // A colon inside the registry's host:port is not a tag.
        Some((p, t)) if !t.contains('/') => (p, Some(t)),
        _ => (reference, None),
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    match tag {
        Some("latest") | None => format!("{name}, the latest version"),
        Some("<none>") => format!("{name}, with no version"),
        Some(t) => format!("{name}, version {t}"),
    }
}

pub fn parse_volumes(text: &str) -> Vec<Volume> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts = columns(line, 3);
            Volume {
                name: column(&parts, 0),
                driver: column(&parts, 1),
                mount_point: column(&parts, 2),
            }
        })
        .collect()
}

// ── Naming one container ─────────────────────────────────────────────

/// Which row a caller means by `needle`, as an index into `rows` of `(id, name)`.
///
/// An exact name or an id prefix wins over a substring, so `web` picks the container called
/// `web` over `webhook-runner` even when the latter is listed first. `None` is a refusal, and
/// the actions above this treat it as one: naming a container that is not here must not be
/// answered with the success of having done nothing to it.
pub fn resolve(rows: &[(String, String)], needle: &str) -> Option<usize> {
    let want = needle.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    rows.iter()
        .position(|(id, name)| name.to_lowercase() == want || id.to_lowercase().starts_with(&want))
        .or_else(|| {
            rows.iter()
                .position(|(_, name)| name.to_lowercase().contains(&want))
        })
}
