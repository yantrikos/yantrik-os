//! What the container runtime's answers mean, tested without docker on the machine. Moved here
//! unchanged from `tests/container-core`, which compiled `runtime.rs` by path; the app is a
//! library now, and these test it as one.
//!
//! The bugs these cover were both invisible to a reader of the Rust alone. Nine mutations ran
//! `let _ = Command::new(runtime_cmd())...output();`, so the control surface answered
//! `{"removed": name}` on an action graded `dangerous` whether the container was gone or the
//! daemon had never been reached. And `list_containers` returned `vec![]` for "docker is not
//! installed", "the daemon is down" and "this machine has no containers" alike, so a mind reading
//! `describe` concluded the machine was empty. Both decisions live in one module with no window
//! and no process in it, which is the module tested here.

use crate::runtime::{
    availability, outcome, parse_containers, parse_images, parse_volumes, resolve,
    Availability, Exit,
};

fn ran(code: i32, stdout: &str, stderr: &str) -> Exit {
    Exit::Ran {
        code: Some(code),
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

// ── The three things an empty list used to mean ──────────────────

#[test]
fn an_empty_listing_from_a_working_runtime_is_an_empty_machine() {
    let exit = ran(0, "", "");
    assert_eq!(availability(&exit), Availability::Ready);
    assert!(parse_containers("").is_empty());
    // Nothing is wrong here, and saying something was would be the mirror of the old bug.
    assert_eq!(availability(&exit).trouble("docker"), None);
    assert_eq!(availability(&exit).state_name(), "ready");
}

#[test]
fn a_missing_binary_is_not_an_empty_machine() {
    let state = availability(&Exit::Missing);
    assert_eq!(state, Availability::Missing);
    assert!(!state.is_ready());
    assert_eq!(state.state_name(), "not_installed");
    assert_eq!(
        state.trouble("docker").unwrap(),
        "docker is not installed on this machine"
    );
}

#[test]
fn a_daemon_that_will_not_answer_says_why() {
    let exit = ran(
        1,
        "",
        "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
         Is the docker daemon running?\n",
    );
    let state = availability(&exit);
    assert_eq!(state.state_name(), "unreachable");
    assert!(!state.is_ready());
    let said = state.trouble("docker").unwrap();
    assert!(said.starts_with("the docker daemon is not reachable: "), "{said}");
    assert!(said.contains("unix:///var/run/docker.sock"), "{said}");
}

#[test]
fn a_runtime_that_would_not_start_at_all_is_unreachable_with_the_reason() {
    let state = availability(&Exit::Unstartable("Permission denied (os error 13)".into()));
    assert_eq!(
        state,
        Availability::Unreachable("Permission denied (os error 13)".into())
    );
}

#[test]
fn a_failure_with_nothing_on_either_stream_still_reports_unreachable() {
    assert_eq!(
        availability(&ran(125, "", "")),
        Availability::Unreachable("it would not answer".into())
    );
}

// ── A mutation reports what the runtime said ─────────────────────

#[test]
fn a_refused_command_carries_the_runtimes_own_words() {
    // The one the survey found: `remove` answered `{"removed": name}` for exactly this.
    let err = outcome(
        "docker",
        &["rm", "-f", "web"],
        ran(1, "", "Error response from daemon: No such container: web\n"),
    )
    .unwrap_err();
    assert_eq!(err, "Error response from daemon: No such container: web");
}

#[test]
fn a_command_that_worked_hands_back_what_it_printed() {
    assert_eq!(
        outcome("docker", &["start", "web"], ran(0, "web\n", "")).unwrap(),
        "web\n"
    );
}

#[test]
fn a_missing_runtime_refuses_the_mutation_rather_than_reporting_it_done() {
    assert_eq!(
        outcome("podman", &["stop", "web"], Exit::Missing).unwrap_err(),
        "podman is not installed on this machine"
    );
    assert_eq!(
        outcome("docker", &["stop", "web"], Exit::Unstartable("os error 13".into()))
            .unwrap_err(),
        "docker could not be started: os error 13"
    );
}

#[test]
fn a_silent_failure_names_the_command_and_the_status() {
    assert_eq!(
        outcome("docker", &["volume", "rm", "data"], ran(125, "", "")).unwrap_err(),
        "`docker volume rm data` exited with status 125"
    );
    assert_eq!(
        outcome(
            "docker",
            &["pull", "nginx"],
            Exit::Ran { code: None, stdout: String::new(), stderr: String::new() },
        )
        .unwrap_err(),
        "`docker pull nginx` was killed"
    );
}

#[test]
fn the_first_line_that_says_anything_is_the_one_reported() {
    // A usage banner is many lines and the leading one is blank as often as not; stdout is
    // the fallback because `pull` writes its progress there and its failure with it.
    let err = outcome("docker", &["rmi", "nginx"], ran(1, "", "\n\nError: image is in use by 2 containers\nSee 'docker rmi --help'.\n")).unwrap_err();
    assert_eq!(err, "Error: image is in use by 2 containers");
    let err = outcome("docker", &["pull", "nope"], ran(1, "Error: manifest unknown\n", "")).unwrap_err();
    assert_eq!(err, "Error: manifest unknown");
}

// ── Reading the listings ─────────────────────────────────────────

const PS: &str = "\
9f2c1b3d4e5f\tweb\tnginx:latest\trunning\tUp 2 hours\t0.0.0.0:8080->80/tcp, 0.0.0.0:8443->443/tcp\t2026-09-18 11:02:14 +0000 UTC
1a2b3c4d5e6f\tdb\tpostgres:16\texited\tExited (0) 3 days ago\t\t2026-09-15 09:41:00 +0000 UTC
";

#[test]
fn a_listing_keeps_the_ports_column_whole() {
    let rows = parse_containers(PS);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "web");
    assert_eq!(rows[0].state, "running");
    assert_eq!(rows[0].image, "nginx:latest");
    // Commas inside one column, which is why the split is by tab and bounded to seven.
    assert_eq!(
        rows[0].ports,
        "0.0.0.0:8080->80/tcp, 0.0.0.0:8443->443/tcp"
    );
    assert_eq!(rows[0].created, "2026-09-18 11:02:14 +0000 UTC");
    assert_eq!(rows[1].state, "exited");
    assert_eq!(rows[1].ports, "");
    assert_eq!(rows[1].status_text, "Exited (0) 3 days ago");
}

#[test]
fn blank_lines_and_carriage_returns_do_not_become_rows() {
    let text = "\n9f2c\tweb\tnginx\trunning\tUp 2 hours\t\t2026-09-18\r\n\n   \n";
    let rows = parse_containers(text);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].created, "2026-09-18");
}

#[test]
fn a_row_with_no_state_column_reads_as_stopped_not_as_blank() {
    // The screen colours anything that is not "running" as stopped; an empty string would
    // have drawn as a fourth, silent state.
    let rows = parse_containers("9f2c\tweb\tnginx\n");
    assert_eq!(rows[0].state, "stopped");
    assert_eq!(rows[0].status_text, "");
}

#[test]
fn images_and_volumes_read_the_same_way() {
    let images = parse_images(
        "sha256:aa\tnginx:latest\t187MB\t2026-09-01 10:00:00 +0000 UTC\nsha256:bb\t<none>:<none>\t92MB\t2026-08-30 08:00:00 +0000 UTC\n",
    );
    assert_eq!(images.len(), 2);
    assert_eq!(images[0].repo_tag, "nginx:latest");
    assert_eq!(images[1].size_text, "92MB");

    let volumes = parse_volumes("pgdata\tlocal\t/var/lib/docker/volumes/pgdata/_data\n");
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0].name, "pgdata");
    assert_eq!(volumes[0].driver, "local");
    assert_eq!(volumes[0].mount_point, "/var/lib/docker/volumes/pgdata/_data");
}

// ── Naming one container ─────────────────────────────────────────

fn rows() -> Vec<(String, String)> {
    vec![
        ("9f2c1b3d4e5f".into(), "webhook-runner".into()),
        ("1a2b3c4d5e6f".into(), "web".into()),
        ("7d8e9f0a1b2c".into(), "db".into()),
    ]
}

#[test]
fn an_exact_name_beats_a_substring_wherever_it_is_in_the_list() {
    assert_eq!(resolve(&rows(), "web"), Some(1));
    assert_eq!(resolve(&rows(), "WEB"), Some(1));
    assert_eq!(resolve(&rows(), " web "), Some(1));
    assert_eq!(resolve(&rows(), "webhook"), Some(0));
}

#[test]
fn an_id_prefix_names_a_container_the_way_docker_does() {
    assert_eq!(resolve(&rows(), "7d8e"), Some(2));
    assert_eq!(resolve(&rows(), "7d8e9f0a1b2c"), Some(2));
}

#[test]
fn a_container_that_is_not_here_is_a_refusal_and_not_a_guess() {
    // The assertion the live probe repeats against the running app: acting on a container
    // that does not exist must be refused, never answered with the success of having done
    // nothing to it.
    assert_eq!(resolve(&rows(), "no-such-container"), None);
    assert_eq!(resolve(&rows(), ""), None);
    assert_eq!(resolve(&rows(), "   "), None);
    assert_eq!(resolve(&[], "web"), None);
}

#[test]
fn an_image_row_opens_with_a_plain_name_not_a_registry_path() {
    use crate::runtime::plain_image_name;
    assert_eq!(plain_image_name("docker.io/library/alpine:latest"), "alpine, the latest version");
    assert_eq!(plain_image_name("ghcr.io/acme/web:1.4"), "web, version 1.4");
    assert_eq!(plain_image_name("localhost:5000/tools"), "tools, the latest version");
    assert_eq!(plain_image_name("<none>:<none>"), "An image with no name");
}
