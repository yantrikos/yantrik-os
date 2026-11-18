//! What the agent rail knows here: the picture on screen, by name and dimensions, both of which
//! the app read off the file.
//!
//! No suggestion: what would be useful here is describing the IMAGE, and that needs a vision
//! model this OS does not attach — the header's Describe button asks about what the file records
//! instead, and says so. Offering a full captioning anyway is the fifty-five-dead-buttons
//! mistake, so this window's rail has no NEXT section and no proposal card at all
//! (image_viewer.slint), and nothing here is ever asked of the Mind.

use yantrik_app::mind::Reach;
use yantrik_app::rail::{ContextItem, HasRail, Prompt, Proposal, Rail, Source, Suggestion};

use crate::model::Model;

impl Source for Model {
    fn context(&self) -> Vec<ContextItem> {
        if self.shown.name.is_empty() {
            return Vec::new();
        }
        vec![ContextItem {
            id: "file".into(),
            label: self.shown.name.clone(),
            detail: self.shown.info.dimensions.clone(),
            source: "file".into(),
        }]
    }

    fn suggestions(&self, _reach: Reach) -> Vec<Suggestion> {
        Vec::new()
    }

    fn prompt(&self, _suggestion: &str) -> Option<Prompt> {
        None
    }

    fn proposal(&self, _suggestion: &str, _answer: &str) -> Proposal {
        Proposal::default()
    }
}

impl HasRail for Model {
    fn rail(&self) -> &Rail {
        &self.rail
    }

    fn rail_mut(&mut self) -> &mut Rail {
        &mut self.rail
    }
}
