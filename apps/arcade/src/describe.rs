//! The Mind's view of Arcade: a projection of the library as last read, in the same keys and words
//! the app has always answered, so `yos check arcade` reads the same.
//!
//! Built from the model alone: nothing here touches the disk. The lists are the lane's last
//! reading (`Command::Reload` brings it up to date when the window is brought forward).

use yantrik_app::control::View;

use crate::model::Model;
use crate::spec;

/// How many rows of each list `describe` carries. The state is a glance, not a transcript.
pub const LISTING_CAP: usize = 20;

pub fn view(m: &Model) -> View {
    let games = &m.listing.games;
    let characters = &m.listing.characters;

    let mut summary = format!("Arcade \u{2014} {} games, {} characters", games.len(), characters.len());
    if let Some(job) = m.jobs.running().first() {
        summary = format!("Arcade \u{2014} {} of {} running", job.action, job.subject);
    }

    View::new(summary)
        .with("library", m.library.root().display().to_string())
        .with("selected", m.game_name.clone())
        .with(
            "games",
            games
                .iter()
                .take(LISTING_CAP)
                .map(|g| {
                    serde_json::json!({
                        "slug": g.slug, "title": g.title, "built": g.built,
                        "last_build": g.last_build, "verification": g.verification,
                    })
                })
                .collect::<Vec<_>>(),
        )
        .with(
            "characters",
            characters
                .iter()
                .take(LISTING_CAP)
                .map(|c| serde_json::json!({ "name": c.name, "slug": c.slug, "archetype": c.archetype }))
                .collect::<Vec<_>>(),
        )
        .with("jobs", m.jobs.json())
        // The grammar, read off the same constants the validator enforces. It lives in the state
        // as well as in the argument text because a caller that has already been refused once
        // reads `describe` next, and that read should end the guessing rather than continue it.
        .with(
            "spec_grammar",
            serde_json::json!({
                "vocabularies": spec::vocabularies(),
                "ranges": spec::ranges(),
                "note": NOTE,
            }),
        )
}

/// Word for word as it has always been published, spaces and all.
const NOTE: &str = "Every enumerated field and its words, and every numeric field and its                          range, exactly as new_character and new_game enforce them.";
