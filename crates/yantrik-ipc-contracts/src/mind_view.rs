//! How an app learns it is drawn in Mind View, where an agent reads the screen as text.

/// Set (to `1`) by the shell on every app it launches onto a mind's display
/// (`MindDisplay::mind_view`). An app reads it at start; it can only turn masking on.
pub const FOR_MIND_VAR: &str = "YANTRIK_FOR_MIND";

/// The person's own Wayland display. A window on any other is Mind View's, whatever the marker
/// says.
pub const PERSON_DISPLAY: &str = "wayland-0";

/// Whose windows an app of the framework has open, as the app's one instance reports them (#963,
/// #972 S1). The instance writes it beside its socket on every change to its windows, and removes
/// it when it ends. The shell lists the person's windows from it, never from launch pids: one
/// instance holds the person's windows and a Mind's together.
pub mod window_owners {
    /// The report's file name for the instance named `instance` (`notes`, `notes.work`), in the
    /// session's socket directory.
    pub fn file_name(instance: &str) -> String {
        format!("{instance}.windows")
    }

    /// The largest report either side reads.
    pub const MAX_BYTES: usize = 4096;

    /// One instance's windows.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct Report {
        /// The instance's pid.
        pub pid: u32,
        /// How many of its windows are open on the person's display.
        pub person: u32,
        /// The Mind displays it has a window open on.
        pub minds: Vec<String>,
    }

    impl Report {
        /// One line: `<pid> <person windows> <mind displays, comma separated, or ->`.
        pub fn encode(&self) -> String {
            let minds = if self.minds.is_empty() { "-".to_string() } else { self.minds.join(",") };
            format!("{} {} {}\n", self.pid, self.person, minds)
        }

        /// The line [`Report::encode`] writes; `None` for anything else.
        pub fn parse(text: &str) -> Option<Report> {
            if text.len() > MAX_BYTES {
                return None;
            }
            let mut words = text.trim().split(' ');
            let pid = words.next()?.parse().ok().filter(|p| *p > 0)?;
            let person = words.next()?.parse().ok()?;
            let minds = match words.next()? {
                "-" => Vec::new(),
                list => list.split(',').map(str::to_string).collect::<Vec<_>>(),
            };
            let named = |d: &String| !d.is_empty() && d.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if words.next().is_some() || !minds.iter().all(named) {
                return None;
            }
            Some(Report { pid, person, minds })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_report_reads_back_as_written_and_nothing_else_reads() {
            let r = Report { pid: 42, person: 1, minds: vec!["wayland-1".into()] };
            assert_eq!(Report::parse(&r.encode()), Some(r));
            let none = Report { pid: 7, person: 0, minds: vec![] };
            assert_eq!(Report::parse(&none.encode()), Some(none));
            for bad in ["", "0 1 -", "x 1 -", "1 1", "1 1 - extra", "1 1 way land", "1 1 a/b", "1 -1 -"] {
                assert_eq!(Report::parse(bad), None, "{bad}");
            }
            assert_eq!(file_name("notes"), "notes.windows");
        }
    }
}
