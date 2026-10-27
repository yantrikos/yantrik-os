//! The matrix for the shell's lists, played for #978 step 1 (yantrik-mind-3e's three open orders
//! and two close orders): three orders in which an agent and the person reach one app, and two
//! in which it ends. At every step the person's `windows` lists the app exactly when the person
//! has a window of it open, and Mind View's apps never list it, because no framework app gives a
//! Mind a window.
//!
//! The shell is played as it runs: the dock's launch and reap calls for the person's launches, no
//! record at all for an instance an agent's act started with no window, Mind View's pid list, and
//! the instance's own report of its windows.

use std::cell::RefCell;
use std::collections::HashSet;

use super::*;

const NOTES: &str = "notes";
const BIN: &str = "yantrik-notes";

#[derive(Default)]
struct Instance {
    pid: u32,
    /// Started by the person's launch, so the shell reaps it and holds its record.
    shells_child: bool,
    person: u32,
    /// An agent has asked it something and it has not gone idle: it outlives its last window.
    serving: bool,
}

#[derive(Default)]
struct Shell {
    seats: Seats,
    /// Mind View's apps: the pids launched there, which is never a framework app.
    mind_view: Vec<(u32, String)>,
    alive: RefCell<HashSet<u32>>,
    instance: Option<Instance>,
    next_pid: u32,
}

impl Shell {
    fn new() -> Shell {
        Shell { next_pid: 100, ..Shell::default() }
    }

    fn spawn(&mut self) -> u32 {
        self.next_pid += 1;
        self.alive.borrow_mut().insert(self.next_pid);
        self.next_pid
    }

    fn record(pid: u32) -> Record {
        Record { app_id: NOTES.into(), pid, binary: BIN.into(), since_unix: 0 }
    }

    fn launched(&mut self, pid: u32) {
        let alive = self.alive.borrow().clone();
        self.seats.launched(Self::record(pid), |p| alive.contains(&p));
    }

    /// The reaper: a child of the shell exited.
    fn exited(&mut self, pid: u32) {
        self.alive.borrow_mut().remove(&pid);
        let alive = self.alive.borrow().clone();
        let report = self.report();
        self.seats.exited(NOTES, pid, |_| report.clone(), |p| alive.contains(&p));
    }

    fn report(&self) -> Option<Report> {
        let i = self.instance.as_ref()?;
        self.alive.borrow().contains(&i.pid).then(|| Report { pid: i.pid, person: i.person })
    }

    /// An agent's act or describe: the shell starts the instance with no window and leaves no
    /// launch record, since nothing is on the person's desktop. One already running just answers.
    fn agent_serves(&mut self) {
        match &mut self.instance {
            Some(i) => i.serving = true,
            None => {
                let pid = self.spawn();
                self.instance = Some(Instance { pid, shells_child: false, person: 0, serving: true });
            }
        }
    }

    /// A Mind's `open_app` of the app: refused before anything is spawned or recorded.
    fn mind_tries_to_open(&mut self) {
        let installed = vec![crate::apps::DesktopEntry {
            app_id: BIN.into(),
            exec: format!("/opt/yantrik/bin/{BIN}"),
            name: "Notes".into(),
            ..Default::default()
        }];
        let said = crate::agent_open::may_launch(NOTES, &installed).unwrap_err();
        assert!(said.ends_with("use its actions and os_describe notes instead."), "{said}");
    }

    fn person_opens(&mut self) {
        match self.instance.take() {
            None => {
                let pid = self.spawn();
                self.launched(pid);
                self.instance = Some(Instance { pid, shells_child: true, person: 1, serving: false });
            }
            Some(mut i) => {
                // A second copy hands over to the instance, which opens the person's window on
                // its own display, and exits.
                let copy = self.spawn();
                self.launched(copy);
                i.person = 1;
                self.instance = Some(i);
                self.exited(copy);
            }
        }
    }

    fn person_closes(&mut self) {
        self.instance.as_mut().expect("the person has a window").person = 0;
        self.instance_ends_if_unneeded();
    }

    /// No agent has asked for a while: the instance stops serving, and ends if nothing else keeps it.
    fn goes_idle(&mut self) {
        if let Some(i) = self.instance.as_mut() {
            i.serving = false;
        }
        self.instance_ends_if_unneeded();
    }

    fn instance_ends_if_unneeded(&mut self) {
        let Some(i) = &self.instance else { return };
        if i.person > 0 || i.serving {
            return;
        }
        let (pid, child) = (i.pid, i.shells_child);
        self.instance = None;
        if child {
            self.exited(pid);
        } else {
            self.alive.borrow_mut().remove(&pid);
        }
    }

    /// `describe shell`: whether `windows` lists the app, and whether `mind_view.apps` does.
    fn described(&self) -> (bool, bool) {
        let in_mind_view: HashSet<u32> = self.mind_view.iter().map(|(p, _)| *p).collect();
        let alive = self.alive.borrow().clone();
        let report = self.report();
        let windows = self.seats.persons(&in_mind_view, |p| alive.contains(&p), |_| report.clone());
        (windows.iter().any(|r| r.app_id == NOTES), self.mind_view.iter().any(|(_, id)| id == NOTES))
    }

    /// What it should say: the person's window, and never a Mind's.
    fn truth(&self) -> (bool, bool) {
        (self.instance.as_ref().is_some_and(|i| i.person > 0), false)
    }

    fn step(&mut self, what: &str, act: fn(&mut Shell)) {
        act(self);
        assert_eq!(self.described(), self.truth(), "after `{what}`: (windows has Notes, mind_view.apps has Notes)");
    }
}

fn run(steps: &[(&str, fn(&mut Shell))]) {
    let mut shell = Shell::new();
    for (what, act) in steps {
        shell.step(what, *act);
    }
    assert!(shell.instance.is_none(), "everything ended, so no instance is left");
    assert!(shell.seats.records(|p| shell.alive.borrow().contains(&p)).is_empty(), "no stale record");
}

#[test]
fn order_1_the_agent_first_then_the_person_closes_then_the_instance_goes_idle() {
    run(&[
        ("agent serves", Shell::agent_serves),
        ("person opens", Shell::person_opens),
        ("person closes", Shell::person_closes),
        ("idle", Shell::goes_idle),
    ]);
}

#[test]
fn order_1_the_agent_first_then_idle_under_the_open_window_then_the_person_closes() {
    run(&[
        ("agent serves", Shell::agent_serves),
        ("person opens", Shell::person_opens),
        ("idle", Shell::goes_idle),
        ("person closes", Shell::person_closes),
    ]);
}

#[test]
fn order_2_the_person_first_then_the_agent_then_the_person_closes_first() {
    run(&[
        ("person opens", Shell::person_opens),
        ("agent serves", Shell::agent_serves),
        ("person closes", Shell::person_closes),
        ("idle", Shell::goes_idle),
    ]);
}

#[test]
fn order_2_the_person_first_then_the_agent_then_it_goes_idle_first() {
    run(&[
        ("person opens", Shell::person_opens),
        ("agent serves", Shell::agent_serves),
        ("idle", Shell::goes_idle),
        ("person closes", Shell::person_closes),
    ]);
}

#[test]
fn order_3_a_minds_open_is_refused_then_the_agent_then_the_person_closing_first() {
    run(&[
        ("mind tries to open", Shell::mind_tries_to_open),
        ("agent serves", Shell::agent_serves),
        ("person opens", Shell::person_opens),
        ("person closes", Shell::person_closes),
        ("idle", Shell::goes_idle),
    ]);
}

#[test]
fn order_3_a_minds_open_is_refused_then_the_agent_then_idle_first() {
    run(&[
        ("mind tries to open", Shell::mind_tries_to_open),
        ("agent serves", Shell::agent_serves),
        ("person opens", Shell::person_opens),
        ("idle", Shell::goes_idle),
        ("mind tries to open", Shell::mind_tries_to_open),
        ("person closes", Shell::person_closes),
    ]);
}

/// A windowless instance is not a window, even if a launch record still names its pid.
#[test]
fn a_windowless_instance_is_not_a_window_even_if_a_launch_record_names_it() {
    let mut shell = Shell::new();
    shell.person_opens();
    // The person closes, and an agent keeps the instance alive: the record's pid is still alive.
    shell.instance.as_mut().unwrap().person = 0;
    shell.instance.as_mut().unwrap().serving = true;
    assert_eq!(shell.described(), (false, false), "the record's pid is alive, the person has no window");
    shell.person_opens();
    assert_eq!(shell.described(), (true, false));
}

/// A record whose app has no report yet (still starting, or an older build) is the launch pid's.
#[test]
fn an_app_with_no_report_yet_is_listed_by_its_launch() {
    let mut seats = Seats::default();
    seats.launched(Shell::record(7), |_| true);
    let none = HashSet::new();
    assert_eq!(seats.persons(&none, |_| true, |_| None).len(), 1);
    assert!(seats.persons(&none, |_| true, |_| Some(Report { pid: 7, person: 0 })).is_empty());
}

#[test]
fn a_report_is_read_only_from_a_small_file_of_this_account_naming_a_live_process() {
    let dir = std::env::temp_dir().join(format!("yantrik-seats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let notes = crate::framework::by_id("notes").unwrap();
    let me = std::process::id();
    assert_eq!(read_report(&dir, notes, |_| true), None, "no file");
    let report = Report { pid: me, person: 1 };
    std::fs::write(dir.join("notes.windows"), report.encode()).unwrap();
    assert_eq!(read_report(&dir, notes, |p| p == me), Some(report));
    assert_eq!(read_report(&dir, notes, |_| false), None, "its process is gone");
    std::fs::write(dir.join("notes.windows"), "not a report").unwrap();
    assert_eq!(read_report(&dir, notes, |_| true), None);
    std::fs::write(dir.join("notes.windows"), "x".repeat(5000)).unwrap();
    assert_eq!(read_report(&dir, notes, |_| true), None, "too long to be one");
    let _ = std::fs::remove_dir_all(&dir);
}
