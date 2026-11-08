use super::*;

fn host(systemd: bool, run: Option<&str>, dir: Option<&str>, bus: Option<&str>) -> Host {
    Host {
        systemd,
        systemd_run: run.map(String::from),
        runtime_dir: dir.map(String::from),
        bus: bus.map(String::from),
    }
}

const RUN: &str = "/usr/bin/systemd-run";

#[test]
fn a_program_goes_to_the_minds_slice_in_a_scope_of_its_own() {
    let h = host(true, Some(RUN), Some("/run/user/1000"), Some("unix:path=/run/user/1000/bus"));
    let l = plan("job-ab12", "/bin/bash", &h, true).unwrap();
    assert_eq!(l.program, RUN);
    assert_eq!(l.unit.as_deref(), Some("mind-job-ab12.scope"));
    assert_eq!(&l.args[..4], ["--user", "--scope", "--quiet", "--collect"]);
    assert!(l.args.contains(&"--slice=yantrik-minds.slice".to_string()));
    assert!(l.args.contains(&"--unit=mind-job-ab12".to_string()));
    assert_eq!(l.args.last().map(String::as_str), Some("/bin/bash"));
    assert!(l.env.contains(&("XDG_RUNTIME_DIR".into(), "/run/user/1000".into())));
    assert!(l.env.iter().any(|(k, _)| k == "DBUS_SESSION_BUS_ADDRESS"));
}

#[test]
fn a_stripped_program_is_not_handed_the_way_to_the_user_manager() {
    let l = plan("job-1", "/bin/bash", &host(true, Some(RUN), Some("/run/user/1000"), None), true).unwrap();
    let joined = l.args.join(" ");
    assert!(
        joined.contains("-- /usr/bin/env -u XDG_RUNTIME_DIR -u DBUS_SESSION_BUS_ADDRESS -u INVOCATION_ID /bin/bash"),
        "{joined}"
    );
    assert!(!l.env.iter().any(|(k, _)| k == "DBUS_SESSION_BUS_ADDRESS"));
}

#[test]
fn an_app_keeps_its_runtime_directory() {
    let h = host(true, Some(RUN), Some("/run/user/1000"), None);
    let l = plan("app-files-1", "/opt/yantrik/bin/files", &h, false).unwrap();
    assert!(l.args.join(" ").ends_with("-- /opt/yantrik/bin/files"), "{:?}", l.args);
    assert!(!l.args.contains(&"-u".to_string()));
}

#[test]
fn on_a_systemd_host_without_a_user_manager_the_launch_is_refused() {
    for (run, dir) in [(None, Some("/run/user/1000")), (Some(RUN), None), (Some(RUN), Some(""))] {
        let why = plan("job-1", "/bin/bash", &host(true, run, dir, None), true).unwrap_err();
        assert!(why.contains("was not started") && why.contains("no user manager"), "{why}");
    }
}

#[test]
fn without_systemd_the_program_starts_directly() {
    let l = plan("job-1", "/bin/bash", &host(false, None, None, None), true).unwrap();
    assert_eq!(l, Launch { program: "/bin/bash".into(), args: vec![], env: vec![], unit: None });
}

#[test]
fn a_unit_name_carries_nothing_a_unit_name_cannot() {
    assert_eq!(unit_name("job-ab/../12 x"), "mind-job-ab____12_x.scope");
}

#[test]
fn fresh_ids_differ() {
    assert_ne!(fresh_id("app-files"), fresh_id("app-files"));
}

#[test]
fn a_wrapped_command_keeps_its_arguments_environment_and_directory() {
    let mut original = Command::new("/opt/yantrik/bin/files");
    original.args(["--open", "/tmp/x"]).env("A", "1").env_remove("B").current_dir("/tmp");
    // The host of the test may or may not run systemd; either way the wrap keeps the command's own.
    let Ok((wrapped, unit)) = wrap(&original, "app-files-1", false) else { return };
    let args: Vec<_> = wrapped.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    if unit.is_some() {
        assert!(args.join(" ").contains("-- /opt/yantrik/bin/files --open /tmp/x"), "{args:?}");
    } else {
        assert_eq!(args, ["--open", "/tmp/x"]);
    }
    let envs: Vec<_> = wrapped.get_envs().collect();
    assert!(envs.iter().any(|(k, v)| *k == "A" && v.is_some()));
    assert!(envs.iter().any(|(k, v)| *k == "B" && v.is_none()));
    assert_eq!(wrapped.get_current_dir(), Some(std::path::Path::new("/tmp")));
}
