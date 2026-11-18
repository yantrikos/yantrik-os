//! Whose window to close to make room for another's, in a model that is full (#1257 review, N3
//! and M3).
//!
//! Any program can announce windows, and any number of connections. The share is therefore not
//! per connection: five hundred connections of one program are one program, and its share is one
//! share. Processes are told apart by the pid the listing reads; a connection whose pid is not
//! known (0) shares a bucket with every other that is not.

use std::collections::HashMap;

/// What one connection holds.
#[derive(Debug, Clone, Copy)]
pub struct Holding {
    pub pid: u32,
    pub windows: usize,
    /// When the connection was first learned: a counter, larger is later.
    pub learned: u64,
}

/// The process whose window may be closed to make room for one from process `pid`, or `None`
/// when `pid` already holds its equal share of `cap` windows, or nobody else holds any. It is
/// the process holding the most; of equal ones, the one learned last, so that the application
/// that was there first keeps its windows and a flood of newer ones does not.
pub fn take_from(holdings: impl Iterator<Item = Holding>, pid: u32, cap: usize) -> Option<u32> {
    let mut by_process: HashMap<u32, (usize, u64)> = HashMap::new();
    for held in holdings {
        let entry = by_process.entry(held.pid).or_insert((0, 0));
        entry.0 += held.windows;
        entry.1 = entry.1.max(held.learned);
    }
    let holds = by_process.get(&pid).map_or(0, |entry| entry.0);
    let sharing = by_process.len() + usize::from(!by_process.contains_key(&pid));
    if holds >= cap / sharing.max(1) {
        return None;
    }
    by_process.into_iter().filter(|(other, (windows, _))| *other != pid && *windows > 0).max_by_key(|(_, held)| *held).map(|(other, _)| other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(pid: u32, windows: usize, learned: u64) -> Holding {
        Holding { pid, windows, learned }
    }

    #[test]
    fn a_process_with_its_share_takes_from_nobody() {
        assert_eq!(take_from([held(1, 6, 0), held(2, 6, 1)].into_iter(), 1, 12), None, "6 is its half");
        assert_eq!(take_from([held(1, 5, 0), held(2, 7, 1)].into_iter(), 1, 12), Some(2));
    }

    #[test]
    fn many_connections_of_one_process_are_one_share() {
        let attackers = (0..510).map(|n| held(9, 1, n));
        let honest = [held(5, 2, 600)];
        // The honest process holds 2 of 512, its share of two processes is 256.
        assert_eq!(take_from(attackers.clone().chain(honest), 5, 512), Some(9), "it may take from the program that holds most");
        assert_eq!(take_from(attackers.chain(honest), 9, 512), None, "and the program that holds 510 may not take from it");
    }

    #[test]
    fn of_equal_holders_the_one_learned_last_loses() {
        let list = [held(1, 4, 0), held(2, 4, 5), held(3, 4, 2), held(4, 0, 9)];
        assert_eq!(take_from(list.into_iter(), 4, 16), Some(2));
    }

    #[test]
    fn unknown_pids_share_one_bucket() {
        let list = (0..30).map(|n| held(0, 1, n));
        assert_eq!(take_from(list, 0, 20), None, "thirty windows of unknown processes are one process's, over its share");
    }
}
