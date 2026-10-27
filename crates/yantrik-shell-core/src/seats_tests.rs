//! yantrik-mind-3e's matrix for #972 S1: one Notes instance holding the person's window and a
//! Mind's, opened and closed in every order. At every step the person's `windows` lists Notes
//! exactly when the person has a Notes window, and `mind_view.apps` exactly when a Mind has one.
//!
//! The shell is played as it runs: the dock's launch and reap calls, Mind View's pid list, and the
//! instance's own report of its windows. The instance a Mind's first launch starts (`displays::spawn`
//! in the framework) is not the shell's child, so the shell never reaps it.

use std::cell::RefCell;
use std::collections::HashSet;

use super::*;

const NOTES: &str = "notes";
const BIN: &str = "yantrik-notes";
const MIND_DISPLAY: &str = "wayland-1";

#[derive(Default)]
struct Instance {
    pid: u32,
    /// Started by the shell (the person's launch), so the shell reaps it.
    shells_child: bool,
    person: u32,
    minds: Vec<String>,
}

#[derive(Default)]
struct Shell {
    seats: Seats,
    /// Mind View's apps: the helper pids launched there.
    mind_view: Vec<(u32, String)>,
    alive: RefCell<HashSet<u32>>,
    instance: Option<Instance>,
    next_pid: u32,
    helper: Option<u32>,
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

    fn launched(&mut self, pid: u32, seat: Seat) {
        let alive = self.alive.borrow().clone();
        self.seats.launched(Self::record(pid), seat, |p| alive.contains(&p));
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
        self.alive.borrow().contains(&i.pid).then(|| Report { pid: i.pid, person: i.person, minds: i.minds.clone() })
    }

    fn person_opens(&mut self) {
        if self.instance.is_none() {
            let pid = self.spawn();
            self.launched(pid, Seat::Person);
            self.instance = Some(Instance { pid, shells_child: true, person: 1, minds: Vec::new() });
        } else {
            // A second copy hands over to the instance and exits.
            let copy = self.spawn();
            self.launched(copy, Seat::Person);
            self.instance.as_mut().unwrap().person = 1;
            self.exited(copy);
        }
    }

    fn mind_opens(&mut self) {
        let helper = self.spawn();
        self.launched(helper, Seat::Mind);
        self.mind_view.push((helper, NOTES.into()));
        self.helper = Some(helper);
        if self.instance.is_none() {
            let pid = self.spawn();
            self.instance = Some(Instance { pid, shells_child: false, person: 0, minds: Vec::new() });
        }
        self.instance.as_mut().unwrap().minds = vec![MIND_DISPLAY.into()];
    }

    fn mind_closes(&mut self) {
        let helper = self.helper.take().expect("a Mind's window is open");
        self.instance.as_mut().unwrap().minds.clear();
        self.mind_view.retain(|(p, _)| *p != helper);
        self.exited(helper);
        self.instance_ends_if_windowless();
    }

    fn person_closes(&mut self) {
        self.instance.as_mut().unwrap().person = 0;
        self.instance_ends_if_windowless();
    }

    fn instance_ends_if_windowless(&mut self) {
        let Some(i) = &self.instance else { return };
        if i.person > 0 || !i.minds.is_empty() {
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

    /// `describe shell`: whether `windows` lists Notes, and whether `mind_view.apps` does.
    fn described(&self) -> (bool, bool) {
        let in_mind_view: HashSet<u32> = self.mind_view.iter().map(|(p, _)| *p).collect();
        let alive = self.alive.borrow().clone();
        let report = self.report();
        let windows = self.seats.persons(&in_mind_view, |p| alive.contains(&p), |_| report.clone());
        (windows.iter().any(|r| r.app_id == NOTES), self.mind_view.iter().any(|(_, id)| id == NOTES))
    }

    /// What it should say: what the person and the Mind really have open.
    fn truth(&self) -> (bool, bool) {
        let i = self.instance.as_ref();
        (i.is_some_and(|i| i.person > 0), i.is_some_and(|i| !i.minds.is_empty()))
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
}

#[test]
fn case_1a_the_mind_opens_then_the_person_then_the_mind_closes_first() {
    run(&[
        ("mind opens", Shell::mind_opens),
        ("person opens", Shell::person_opens),
        ("mind closes", Shell::mind_closes),
        ("person closes", Shell::person_closes),
    ]);
}

#[test]
fn case_1b_the_mind_opens_then_the_person_then_the_person_closes_first() {
    run(&[
        ("mind opens", Shell::mind_opens),
        ("person opens", Shell::person_opens),
        ("person closes", Shell::person_closes),
        ("mind closes", Shell::mind_closes),
    ]);
}

#[test]
fn case_2a_the_person_opens_then_the_mind_then_the_mind_closes_first() {
    run(&[
        ("person opens", Shell::person_opens),
        ("mind opens", Shell::mind_opens),
        ("mind closes", Shell::mind_closes),
        ("person closes", Shell::person_closes),
    ]);
}

#[test]
fn case_2b_the_person_opens_then_the_mind_then_the_person_closes_first() {
    run(&[
        ("person opens", Shell::person_opens),
        ("mind opens", Shell::mind_opens),
        ("person closes", Shell::person_closes),
        ("mind closes", Shell::mind_closes),
    ]);
}

#[test]
fn case_3_the_mind_opens_and_closes_then_the_person_opens_and_nothing_stale_is_left() {
    run(&[
        ("mind opens", Shell::mind_opens),
        ("mind closes", Shell::mind_closes),
        ("person opens", Shell::person_opens),
        ("person closes", Shell::person_closes),
    ]);
}

#[test]
fn a_report_is_read_only_from_a_small_file_of_this_account_naming_a_live_process() {
    let dir = std::env::temp_dir().join(format!("yantrik-seats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let notes = crate::framework::by_id("notes").unwrap();
    let me = std::process::id();
    assert_eq!(read_report(&dir, notes, |_| true), None, "no file");
    let report = Report { pid: me, person: 1, minds: vec![MIND_DISPLAY.into()] };
    std::fs::write(dir.join("notes.windows"), report.encode()).unwrap();
    assert_eq!(read_report(&dir, notes, |p| p == me), Some(report));
    assert_eq!(read_report(&dir, notes, |_| false), None, "its process is gone");
    std::fs::write(dir.join("notes.windows"), "not a report").unwrap();
    assert_eq!(read_report(&dir, notes, |_| true), None);
    std::fs::write(dir.join("notes.windows"), "x".repeat(5000)).unwrap();
    assert_eq!(read_report(&dir, notes, |_| true), None, "too long to be one");
    let _ = std::fs::remove_dir_all(&dir);
}
