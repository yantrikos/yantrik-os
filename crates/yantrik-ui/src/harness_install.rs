//! Installing and starting a harness, as a job whose output the person can watch.
//!
//! One thing the Harnesses page could not do before was change anything. It listed what had
//! attached, and a mind that needed an `npm install` or a unit started was a row that did not
//! exist. So the page grew two buttons, and both are jobs rather than calls: an `npm install -g`
//! takes half a minute on a good connection, and a button that freezes the settings screen for
//! thirty seconds with nothing on it is a button people press twice.
//!
//! # Streamed, because that is the only honest progress
//!
//! There is no way to know how far through `npm install` is. What there is, is what it is saying,
//! so that is what the row shows: the last few lines of the command's own output, updated as they
//! arrive. `crate::harness_catalogue::JobView` is what a row reads, and it says `running` until
//! the process exits — which is why a row mid-install says "Installing…" and not "Not installed".
//!
//! # Run as the person, with the person's own environment
//!
//! `sh -lc`, a login shell, with `~/.local/bin` put first on its PATH: that is where every
//! installer in harnesses/lib/install puts what it installs (Node included), and a shell started
//! from a session manager does not have it. A command that works when the person types it and
//! fails from a button is worse than no button.
//!
//! # What is not here
//!
//! Nothing writes a config file. "Needs setup" names a path and stops: the file holds an endpoint
//! and the name of a variable holding a key, both of which are the person's to write, and a
//! desktop that offered to fill one in would be a desktop asking for a key it has nowhere to put.
//! Nothing in this module reads one either, so no output it can produce contains one.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use crate::harness_catalogue::{shell_quote, Install, JobKind, JobView, Manifest, JOB_PATH_PRELUDE, LOG_LINES};

struct Job {
    kind: JobKind,
    doing: String,
    lines: Vec<String>,
    running: bool,
    error: String,
}

impl Job {
    fn view(&self) -> JobView {
        JobView {
            kind: self.kind,
            running: self.running,
            doing: self.doing.clone(),
            log: self.lines.join("\n"),
            error: self.error.clone(),
        }
    }
}

#[derive(Default)]
struct Board {
    jobs: HashMap<String, Job>,
}

fn board() -> &'static Mutex<Board> {
    static BOARD: OnceLock<Arc<Mutex<Board>>> = OnceLock::new();
    BOARD.get_or_init(|| Arc::new(Mutex::new(Board::default())))
}

fn with_board<T>(f: impl FnOnce(&mut Board) -> T) -> T {
    let mut guard = board().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// What every harness with a job has been doing, for the catalogue to fold into its rows.
pub fn views() -> HashMap<String, JobView> {
    with_board(|board| board.jobs.iter().map(|(id, job)| (id.clone(), job.view())).collect())
}

/// `true` while any job is running, so the refresh can keep polling while something is happening
/// and stay quiet when nothing is.
pub fn busy() -> bool {
    with_board(|board| board.jobs.values().any(|j| j.running))
}

/// Run the manifest's install command for `id`.
///
/// Returns the command that was started, so the caller can say what it ran — a person who
/// pressed a button that changes their machine is owed the sentence it ran.
pub fn install(manifest: &Manifest) -> Result<String, String> {
    install_then(manifest, None)
}

/// [`install`], with `worked` run once the command has succeeded and before the job is reported
/// finished, so whoever waits for the row to settle sees what `worked` did. An `Err` from it is
/// put on the row. Never run when the install failed.
pub fn install_then(manifest: &Manifest, worked: Option<Worked>) -> Result<String, String> {
    install_then_or(manifest, worked, None)
}

/// [`install_then`], with `failed` run when the command did not succeed: the install's own final
/// check can fail after a gateway is already up, and whatever the harness held until then (a
/// memory credential) is not to be left standing on an install that did not finish.
pub fn install_then_or(manifest: &Manifest, worked: Option<Worked>, failed: Option<Failed>) -> Result<String, String> {
    let install: &Install = manifest
        .install
        .as_ref()
        .ok_or_else(|| format!("{} has no install command of its own — read its documentation", manifest.id))?;
    let command = install.command_in(&manifest.dir);
    let doing = if install.doing.is_empty() { command.clone() } else { install.doing.clone() };
    // An install that worked goes straight on to the harness's own setup, where it has one: a
    // Hermes with no model is a Hermes that attaches and cannot answer, and a row that says "run
    // hermes model in a terminal" is a step the person has to find for themselves.
    let then: Option<Then> = manifest.configure.is_some().then(|| {
        let manifest = manifest.clone();
        Box::new(move || {
            if let Err(e) = configure(&manifest) {
                tracing::warn!(harness = %manifest.id, error = %e, "the setup after an install did not open");
            }
        }) as Then
    });
    spawn(&manifest.id, JobKind::Install, &doing, command.clone(), worked, failed, then)?;
    Ok(command)
}

/// Open the harness's own setup in a terminal: its model, its sign-in. The person types into it;
/// nothing here reads what they type or what the harness writes.
pub fn configure(manifest: &Manifest) -> Result<String, String> {
    let configure = manifest
        .configure
        .as_ref()
        .ok_or_else(|| format!("{} has no setup of its own to open", manifest.id))?;
    let command = configure.command_in(&manifest.dir);
    let title = if configure.title.is_empty() { configure.label.as_str() } else { configure.title.as_str() };
    crate::terminal_window::open(title, &format!("{JOB_PATH_PRELUDE}{command}"), &[])?;
    tracing::info!(harness = %manifest.id, "opened the harness's own setup");
    Ok(command)
}

/// What to do once a job has finished well.
type Then = Box<dyn FnOnce() + Send>;

/// What has to be done once an install has worked, before the job is reported finished: an
/// `Err` is the sentence the row shows.
pub type Worked = Box<dyn FnOnce() -> Result<(), String> + Send>;

/// What to do when an install did not succeed.
pub type Failed = Box<dyn FnOnce() + Send>;

/// Enable and start a harness's unit.
///
/// Two steps, and the first one is why this is not simply `systemctl --user enable --now`: the
/// image stages each unit where systemd can see it, but a machine installed before that, or a
/// checkout, has the unit file sitting beside the harness source where systemd will never look.
/// So a unit systemd does not know about is copied into `~/.config/systemd/user` first. That is
/// exactly what each README tells a person to do by hand.
pub fn start(manifest: &Manifest) -> Result<String, String> {
    if manifest.unit.is_empty() {
        return Err(format!(
            "{} is not started by a unit of ours — it attaches when its own service runs",
            manifest.id
        ));
    }
    let staged = manifest.dir.join(&manifest.unit);
    let known = unit_known(&manifest.unit);
    let mut steps: Vec<String> = Vec::new();
    if !known {
        if !staged.exists() {
            return Err(format!(
                "systemd does not know {} and there is no {} beside the harness to install",
                manifest.unit,
                manifest.unit
            ));
        }
        let user_units = crate::harness_catalogue::user_unit_dir();
        steps.push(format!("mkdir -p {}", shell_quote(&user_units.display().to_string())));
        steps.push(format!(
            "cp {} {}",
            shell_quote(&staged.display().to_string()),
            shell_quote(&user_units.display().to_string())
        ));
        steps.push("systemctl --user daemon-reload".to_string());
    }
    steps.push(format!("systemctl --user enable --now {}", shell_quote(&manifest.unit)));
    let command = steps.join(" && ");
    spawn(&manifest.id, JobKind::Start, &format!("starting {}", manifest.unit), command.clone(), None, None, None)?;
    Ok(command)
}

/// Whether systemd can already see a unit by this name.
fn unit_known(unit: &str) -> bool {
    Command::new("systemctl")
        .args(["--user", "show", unit, "--property=LoadState"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("LoadState=loaded"))
        .unwrap_or(false)
}


fn spawn(
    id: &str,
    kind: JobKind,
    doing: &str,
    command: String,
    worked: Option<Worked>,
    failed: Option<Failed>,
    then: Option<Then>,
) -> Result<(), String> {
    // One at a time per harness. Two `npm install -g` for the same package at once is a package
    // directory being written by two processes, and the second button press is never what was
    // meant anyway.
    let already = with_board(|board| match board.jobs.get(id) {
        Some(job) if job.running => Some(job.kind),
        _ => None,
    });
    if let Some(kind) = already {
        return Err(format!("a {} for {id} is already running", kind.verb()));
    }

    with_board(|board| {
        board.jobs.insert(
            id.to_string(),
            Job {
                kind,
                doing: doing.to_string(),
                lines: Vec::new(),
                running: true,
                error: String::new(),
            },
        );
    });

    let child = Command::new("sh")
        .arg("-lc")
        .arg(format!("{JOB_PATH_PRELUDE}{command}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(child) => child,
        Err(e) => {
            let message = format!("could not run it: {e}");
            finish(id, Some(message.clone()));
            return Err(message);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let owner = id.to_string();
    std::thread::Builder::new()
        .name(format!("harness-{}-{id}", kind.verb()))
        .spawn(move || {
            // stderr on its own thread, because npm says most of what it has to say there and a
            // reader that drains stdout first would show none of it until the process exited.
            let watcher = stderr.map(|stream| {
                let owner = owner.clone();
                std::thread::spawn(move || pump(&owner, stream))
            });
            if let Some(stream) = stdout {
                pump(&owner, stream);
            }
            if let Some(watcher) = watcher {
                let _ = watcher.join();
            }
            let outcome = match child.wait() {
                Ok(status) if status.success() => None,
                Ok(status) => Some(match status.code() {
                    Some(code) => format!("it exited {code}"),
                    None => "it was killed".to_string(),
                }),
                Err(e) => Some(format!("could not wait for it: {e}")),
            };
            let succeeded = outcome.is_none();
            if let Some(failed) = failed.filter(|_| !succeeded) {
                failed();
            }
            // Before the job is marked finished, so the row never reads "finished" while what
            // the success was for is still being done, and a failed job never reaches it.
            let afterwards = match worked.filter(|_| succeeded) {
                Some(worked) => worked().err(),
                None => None,
            };
            finish(&owner, outcome);
            if let Some(why) = afterwards {
                tracing::warn!(harness = %owner, error = %why, "an install worked and what follows it did not");
                with_board(|board| {
                    if let Some(job) = board.jobs.get_mut(&owner) {
                        job.error = why;
                    }
                });
            }
            if let Some(then) = then.filter(|_| succeeded) {
                then();
            }
        })
        .map_err(|e| {
            finish(id, Some(format!("no thread to watch it: {e}")));
            format!("no thread to watch it: {e}")
        })?;

    tracing::info!(harness = %id, job = kind.verb(), "running a harness job");
    Ok(())
}

fn pump(id: &str, stream: impl std::io::Read) {
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        // Installers colour their output whether or not it is a terminal (Hermes's does), and
        // redraw progress with carriage returns. The row is text, so it gets what was said.
        let line = crate::agents::model::strip_escapes(&line);
        let line = line.trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        with_board(|board| {
            if let Some(job) = board.jobs.get_mut(id) {
                job.lines.push(line);
                // Only the tail is kept. A row is a card on a settings page, and an npm install
                // that printed nine hundred lines into a `String` held in the UI for the rest of
                // the session would be a leak with a progress bar on it.
                let overflow = job.lines.len().saturating_sub(LOG_LINES);
                if overflow > 0 {
                    job.lines.drain(0..overflow);
                }
            }
        });
    }
}

fn finish(id: &str, error: Option<String>) {
    with_board(|board| {
        if let Some(job) = board.jobs.get_mut(id) {
            job.running = false;
            if let Some(error) = &error {
                job.error = format!("{} failed: {error}", job.kind.verb());
            }
        }
    });
    match error {
        Some(error) => tracing::warn!(harness = %id, error = %error, "a harness job failed"),
        None => tracing::info!(harness = %id, "a harness job finished"),
    }
}

/// Forget a finished job, so the row goes back to whatever the machine now says.
///
/// Called once the harness has attached: keeping "install finished" on a row that is now
/// answering questions would be the page still talking about the past.
pub fn clear_settled(attached: &[String]) {
    with_board(|board| {
        board.jobs.retain(|id, job| job.running || !attached.contains(id));
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::harness_catalogue::Manifest;
    use std::time::{Duration, Instant};

    /// These share one process-wide board, so they take turns rather than racing each other
    /// through the same harness id.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    fn taking_turns() -> std::sync::MutexGuard<'static, ()> {
        ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn manifest(id: &str, command: &str) -> Manifest {
        Manifest {
            id: id.to_string(),
            install: Some(Install { command: command.to_string(), doing: "doing the thing".into() }),
            ..Default::default()
        }
    }

    fn settled(id: &str) -> JobView {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let view = views().remove(id).expect("the job was never put on the board");
            if !view.running {
                return view;
            }
            assert!(Instant::now() < deadline, "the job never finished");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_job_is_running_the_moment_it_is_asked_for() {
        // The owner's requirement, and the one a row depends on: between the click and the
        // finish, the state is "installing" and not "not installed".
        let _turn = taking_turns();
        let id = "job-running";
        install(&manifest(id, "sleep 0.4; echo done")).unwrap();
        let view = views().remove(id).unwrap();
        assert!(view.running);
        assert_eq!(view.doing, "doing the thing");

        let view = settled(id);
        assert!(!view.running);
        assert_eq!(view.error, "");
        assert!(view.log.contains("done"), "{}", view.log);
    }

    #[test]
    fn output_arrives_while_it_is_still_going() {
        let _turn = taking_turns();
        let id = "job-streaming";
        install(&manifest(id, "echo first; sleep 1.5; echo last")).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let view = views().remove(id).unwrap();
            if view.log.contains("first") {
                assert!(view.running, "the first line arrived only after it had finished");
                break;
            }
            assert!(Instant::now() < deadline, "no output before the job ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(settled(id).log.contains("last"));
    }

    #[test]
    fn a_coloured_installer_reaches_the_row_as_plain_text() {
        // Hermes's installer colours every line even into a pipe; the row showed the escapes.
        let _turn = taking_turns();
        let id = "job-colour";
        install(&manifest(id, r"printf '\033[0;32m✓\033[0m uv ready\n'")).unwrap();
        let view = settled(id);
        assert_eq!(view.log, "✓ uv ready");
    }

    #[test]
    fn a_job_finds_what_an_earlier_job_put_in_the_per_user_bin() {
        // Pi's install fetches Node into ~/.local/bin and then runs npm from there; a session
        // PATH that lacks it was "npm: not found" on every fresh machine.
        let _turn = taking_turns();
        let id = "job-user-bin";
        install(&manifest(id, r#"case ":$PATH:" in *":$HOME/.local/bin:"*) echo on-path ;; esac"#)).unwrap();
        assert!(settled(id).log.contains("on-path"));
    }

    #[test]
    fn what_it_says_on_stderr_is_shown_too() {
        // npm says most of what it has to say on stderr, and a row that showed none of it would
        // be a spinner beside a command that had already explained itself.
        let _turn = taking_turns();
        let id = "job-stderr";
        install(&manifest(id, "echo 'npm ERR! 404 Not Found' >&2; exit 1")).unwrap();
        let view = settled(id);
        assert!(view.log.contains("404 Not Found"), "{}", view.log);
        assert_eq!(view.error, "install failed: it exited 1");
    }

    #[test]
    fn only_the_tail_of_a_talkative_command_is_kept() {
        let _turn = taking_turns();
        let id = "job-tail";
        install(&manifest(id, "for i in $(seq 1 200); do echo line $i; done")).unwrap();
        let view = settled(id);
        assert_eq!(view.log.lines().count(), LOG_LINES);
        assert!(view.log.contains("line 200"));
        assert!(!view.log.contains("line 1\n"));
    }

    #[test]
    fn a_second_press_while_it_runs_is_refused_rather_than_run_twice() {
        let _turn = taking_turns();
        let id = "job-twice";
        install(&manifest(id, "sleep 1")).unwrap();
        let again = install(&manifest(id, "sleep 1"));
        assert!(again.unwrap_err().contains("already running"));
        settled(id);
    }

    #[test]
    fn a_harness_with_nothing_to_install_says_so_instead_of_running_an_empty_shell() {
        let _turn = taking_turns();
        let bare = Manifest { id: "openclaw".into(), ..Default::default() };
        let refused = install(&bare).unwrap_err();
        assert!(refused.contains("no install command"), "{refused}");
        assert!(views().get("openclaw").is_none(), "a refusal must not leave a job on the board");
    }

    #[test]
    fn a_harness_with_no_unit_is_never_started() {
        let _turn = taking_turns();
        let bare = Manifest { id: "hermes".into(), ..Default::default() };
        let refused = start(&bare).unwrap_err();
        assert!(refused.contains("its own service"), "{refused}");
    }

    #[test]
    fn a_finished_job_is_forgotten_once_the_harness_is_there() {
        let _turn = taking_turns();
        let id = "job-forgotten";
        install(&manifest(id, "true")).unwrap();
        settled(id);
        clear_settled(&["something-else".to_string()]);
        assert!(views().contains_key(id), "it is not attached; the row still needs its outcome");
        clear_settled(&[id.to_string()]);
        assert!(!views().contains_key(id), "it attached, so the row is about the present now");
    }
}
