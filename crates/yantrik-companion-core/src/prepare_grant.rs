//! The one reach a "Prepare for me" job is given beyond an unattended turn (#1416, slice 5).
//!
//! With nobody there, `reach` refuses every page fetch and, while background research is off, every
//! search. A preparation job the person switched on (Settings → Initiative → "Prepare for me")
//! researches an item, so for the length of one job, on the thread that runs it, exactly two tools
//! are let through the reach rule: `web_search` and `web_fetch`. Nothing else is widened: not
//! `open_url`, not a browser, not a package, not any other fetch. The grant is a guard value: it
//! lives on one thread, ends when dropped (also on a panic), and cannot be carried to another
//! thread. The taint rule (`crate::taint`) still runs for every call.
//!
//! The job's own door (`yantrik_companion::prepare::boundary`) is what decides which URLs and
//! queries are asked; this file only says that, inside a job, these two names are not refused for
//! being unattended.

use std::cell::Cell;

thread_local! {
    static HELD: Cell<usize> = const { Cell::new(0) };
}

/// The two tools a job may reach the web with.
pub const TOOLS: [&str; 2] = ["web_search", "web_fetch"];

/// Held while a preparation job runs on this thread.
pub struct Grant(());

impl Grant {
    /// Start the grant on this thread.
    pub fn enter() -> Self {
        HELD.with(|n| n.set(n.get() + 1));
        Grant(())
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        HELD.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// Whether `name` may reach the web now: only the two tools, only inside a job on this thread.
pub fn permits(name: &str) -> bool {
    HELD.with(|n| n.get() > 0) && TOOLS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_tools_and_only_while_held() {
        assert!(!permits("web_fetch"));
        {
            let _g = Grant::enter();
            assert!(permits("web_search") && permits("web_fetch"));
            assert!(!permits("open_url") && !permits("browser_search") && !permits("http_request") && !permits("git_clone"));
        }
        assert!(!permits("web_search"), "the grant ends with the guard");
    }

    #[test]
    fn it_does_not_follow_the_job_to_another_thread() {
        let _g = Grant::enter();
        assert!(std::thread::spawn(|| permits("web_fetch")).join().unwrap() == false);
    }

    #[test]
    fn the_reach_rule_lets_the_two_through_inside_a_job_and_nothing_else() {
        use crate::reach::check_call;
        let args = serde_json::json!({});
        assert!(check_call("web_fetch", "network", &args).is_err(), "unattended: refused");
        let _g = Grant::enter();
        assert!(check_call("web_fetch", "network", &args).is_ok());
        assert!(check_call("web_search", "browser", &args).is_ok());
        assert!(check_call("open_url", "browser", &args).is_err());
        assert!(check_call("git_clone", "network", &args).is_err());
        assert!(check_call("browser_search", "browser", &args).is_err());
    }
}
