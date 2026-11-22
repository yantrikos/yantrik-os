//! Test-only: a `Modes` or the shell's mode set to a given mode, without naming the person-only
//! setters from a `control*.rs` file (`mind_mode_only_a_person_can_raise_the_mode` scans those).
//! `Modes::new` never builds a bypass, so a test of a bypass sets it as a person's click does.

use crate::mind_mode::{Bypass, Mode, Modes};
use std::time::Instant;

/// A `Modes` in `mode`, bypasses included.
pub(crate) fn modes_in(mode: Mode) -> Modes {
    let mut modes = Modes::new(Mode::Ask);
    modes.person_set_mode(mode, Bypass::Hour, Instant::now(), 0);
    modes
}

/// The shell's own mode, as a person's choice from the menu.
pub(crate) fn shell_mode(mode: Mode) {
    crate::mind_mode::person_set_mode(mode, Bypass::Hour);
}
