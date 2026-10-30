//! The desk, run headless through `yantrik-testing`, with a fake shell.

mod compositor;
mod exposure;
mod following;
mod stopping;

use std::sync::Arc;

use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{Paths, Services};
use yantrik_ipc_contracts::mind_desk::{
    Account, Counts, DeskMind, DeskSnapshot, DeskTool, Evidence, MindState, Narration, Outcome,
};
use yantrik_services::mind_desk::FakeMindDesk;
use yantrik_testing::TestApp;

use crate::{MindDesk, MindDeskApp};

/// A Mind at work: Hermes, running `blender.add_object`, on the person's account.
pub(crate) fn hermes(state: MindState) -> DeskMind {
    DeskMind {
        agent: "hermes:c-1".into(),
        name: "Hermes".into(),
        state,
        since: 100,
        narration: Some(Narration { text: "Shaping the last ascent.".into(), heard: "14:32".into() }),
        tool: Some(DeskTool { name: "blender.add_object".into(), evidence: Evidence::Reported, outcome: Outcome::Running }),
        counts: Counts { calls: 12, failed: 1, interrupted: 1 },
        elapsed_secs: Some(272),
        model: Some("deepseek-v4.1-flash via ollama-cloud".into()),
        account: Account::Person,
    }
}

pub(crate) fn snapshot(tag: u64, mind: Option<DeskMind>) -> DeskSnapshot {
    DeskSnapshot { tag, as_of: "14:32".into(), mind, apps: vec!["Blender".into(), "Terminal".into(), "Files".into()] }
}

/// The desk, started against `shell`, which is kept to read what it was asked.
pub(crate) fn start(shell: FakeMindDesk) -> (TestApp<MindDesk>, Arc<FakeMindDesk>) {
    let shell = Arc::new(shell);
    let services = Services::fakes().with_mind_desk(shell.clone());
    let app = TestApp::<MindDesk>::start_at_with(Paths::temporary(<MindDesk as yantrik_app::app::App>::ID), Vec::new(), Arc::new(FakeMind::new()), services);
    (app, shell)
}

/// Every word the top window shows, in one string.
pub(crate) fn top_words(app: &TestApp<MindDesk>) -> String {
    app.ui(|ui: &MindDeskApp| {
        [
            ui.get_label(),
            ui.get_headline(),
            ui.get_reported(),
            ui.get_confirm_question(),
            ui.get_stop_note(),
            ui.get_strip_heading(),
            ui.get_strip_tool(),
            ui.get_strip_mark(),
            ui.get_strip_stats(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" | ")
    })
}
