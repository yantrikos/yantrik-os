//! What the Mind reads: the one line it reads first, and the state behind it.
//!
//! The summary used to be only the working directory, the one thing about a terminal that a mind
//! can already get from `cd`: it did not say whether the shell was alive, whether a command was
//! running in it, or how many tabs there were to choose from. Where the shell is and what runs in
//! it are the last probe's (`probe.rs`), at most a tenth of a second behind while a job runs.
//!
//! Private mode keeps none of these keys (`tests/private_mode.rs`): the framework answers with
//! the neutral view, because a screen can hold anything the person typed or was shown.

use yantrik_app::control::View;
use yantrik_app::for_mind::shown;

use crate::model::Model;

pub fn view(m: &Model) -> View {
    let notice = m.notice.clone();
    let Some(tab) = m.tab() else {
        let mut summary = "Terminal \u{2014} no shell is open; `new_tab` starts one".to_string();
        if !notice.is_empty() {
            summary.push_str(&format!(" \u{b7} {notice}"));
        }
        return View::new(summary)
            .with("tabs", 0)
            .with("alive", false)
            .with("execution", "interactive_pty")
            .with("notice", notice);
    };
    let session = &tab.session;
    let snap = session.snapshot();
    let busy = tab.facts.running.clone();
    let hide = |text: String| shown(m.for_mind, &text).into_owned();
    let cwd = hide(tab.facts.cwd.display().to_string());
    let mut summary = format!(
        "Terminal \u{2014} {cwd}, {}",
        if !snap.alive {
            snap.exit
                .map(|n| format!("shell exited (status {n}); `new_tab` starts another"))
                .unwrap_or_else(|| "the shell has ended".to_string())
        } else {
            match busy.as_deref().map(|j| hide(j.to_string())) {
                Some(job) => format!("running: {job}"),
                None => "shell idle at a prompt".to_string(),
            }
        }
    );
    if m.tabs.len() > 1 {
        summary.push_str(&format!(", tab {} of {}", m.active + 1, m.tabs.len()));
    }
    if snap.scrollback > 0 {
        summary.push_str(&format!(", scrolled back {} lines", snap.scrollback));
    }
    if !notice.is_empty() {
        summary.push_str(&format!(" \u{b7} {notice}"));
    }
    View::new(summary)
        .with("directory", cwd)
        .with("alive", snap.alive)
        .with("running", busy.map(&hide))
        .with("shell_pid", session.pid())
        .with("tabs", m.tabs.len())
        .with("active_tab", m.active)
        .with(
            "tab_directories",
            m.tabs.iter().map(|t| hide(t.facts.cwd.display().to_string())).collect::<Vec<_>>(),
        )
        .with("execution", "interactive_pty")
        .with("shell_exit_code", snap.exit)
        .with("scrollback", snap.scrollback as i64)
        .with("error", snap.error.map(&hide))
        .with("notice", notice)
        .with("recent_output", hide(snap.text))
}
