//! Waiting for the vault to open, and asking again until a job is done.
//!
//! The vault is shut at every boot that starts locked: its passphrase is typed on the shell's
//! unlock card, after the programs that need it have already asked. So whatever the shut vault
//! refused is not given up on. It is queued here ([`after_open`]) and run once, the next time the
//! vault is opened ([`opened`]). Something that must happen at every opening, such as telling the
//! mail service, subscribes once ([`on_open`]).
//!
//! A job that has to keep asking until nothing is left (a migration) uses [`until_done`]: one pass
//! now, one a minute for an hour in case the vault was only slow to start, and one more at each
//! opening, for as long as the pass says work is left.

use std::sync::{Arc, Mutex};
use std::time::Duration;

type Deferred = Box<dyn FnOnce() + Send>;
type Listener = Arc<dyn Fn() + Send + Sync>;

static DEFERRED: Mutex<Vec<Deferred>> = Mutex::new(Vec::new());
static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

/// Run `work` the next time the vault is opened. For something refused because the vault was
/// shut, such as the companion's reload at start, which happens before the person has signed in.
pub fn after_open(work: impl FnOnce() + Send + 'static) {
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(work));
}

/// Run `work` at every opening of the vault, from now on. The listener runs on the thread that
/// opened the vault, which may be the one that draws the screen, so it hands any real work to a
/// thread of its own.
pub fn on_open(work: impl Fn() + Send + Sync + 'static) {
    LISTENERS.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::new(work));
}

/// The vault was just opened (at sign-in, at a screen unlock, or from the unlock card): run what
/// was waiting for it, once, and tell the listeners.
///
/// The queue is taken out before anything runs, so a job that queues itself again for the next
/// opening neither deadlocks nor runs twice in this one.
pub fn opened() {
    let waiting = std::mem::take(&mut *DEFERRED.lock().unwrap_or_else(|e| e.into_inner()));
    for work in waiting {
        work();
    }
    let listeners: Vec<Listener> = LISTENERS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for listener in listeners {
        listener();
    }
}

/// How many jobs are waiting for the next opening. For tests.
pub fn waiting() -> usize {
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).len()
}

/// Drop everything waiting for an opening, without running it. For tests.
pub fn forget_waiting() {
    DEFERRED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// One pass of a job. It is told when it runs (`"start"`, `"retry"`, `"unlock"`, for the log) and
/// answers whether work is left.
pub type Pass = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Ask for `pass` again at the next opening, and at the one after that, for as long as it says
/// work is left. An unlock at any time, even after the timer has given up, reaches the vault.
pub fn until_open_finishes(pass: Pass) {
    after_open(move || {
        if pass("unlock") {
            until_open_finishes(pass);
        }
    });
}

/// The first pass, and the wait for the vault to open when it left work behind, whether the vault
/// was shut or did not answer (the companion's worker may still be starting).
pub fn begin(pass: &Pass) -> bool {
    let left = pass("start");
    if left {
        until_open_finishes(pass.clone());
    }
    left
}

/// [`begin`] on a thread of its own, then `times` more passes `every` apart for as long as work is
/// left. Err when there is no thread to run on.
pub fn until_done(name: &str, pass: Pass, every: Duration, times: usize) -> std::io::Result<()> {
    std::thread::Builder::new().name(name.to_string()).spawn(move || {
        let mut left = begin(&pass);
        for _ in 0..times {
            if !left {
                return;
            }
            std::thread::sleep(every);
            left = pass("retry");
        }
    })?;
    Ok(())
}
