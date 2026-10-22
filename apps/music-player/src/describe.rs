//! The Mind's view of Music: a projection of the committed model, in the same keys and words the
//! app has always answered, so `yos check music` reads the same.

use serde_json::{json, Value};
use yantrik_app::control::{agent_is_calling, View};

use crate::library;
use crate::model::{Model, PlayerState};
use crate::seen;

/// How many rows of the library list `describe` names.
const LISTED: usize = 50;

pub fn view(m: &Model) -> View {
    // Who is reading is the dispatch's to say (the caller and the token it installed), and
    // `describe` is read inside it. An agent is told only the paths it may see (`seen.rs`).
    let agent = agent_is_calling();
    let o = m.observed();
    let loaded = o.path.is_some() && !o.idle && !o.gone;
    let current = m.queue.current().filter(|t| o.path.as_deref() == Some(t.path.as_str()));
    let title = current.map(|t| t.title.clone()).or_else(|| o.path.as_deref().map(library::title_of)).filter(|_| loaded);
    let mut summary = match &title {
        Some(t) if o.playing() => format!(
            "Music \u{2014} playing \u{201c}{t}\u{201d} ({} of {})",
            library::clock(o.time_pos),
            library::clock(o.duration)
        ),
        Some(t) => format!("Music \u{2014} paused on \u{201c}{t}\u{201d} at {}", library::clock(o.time_pos)),
        None => "Music \u{2014} nothing playing".to_string(),
    };
    summary.push_str(&format!(", library {}", library::count(m.library.len(), "track", "tracks")));
    let scanning = m.scans.running > 0;
    if scanning {
        summary.push_str(" \u{b7} scanning");
    }
    if !m.notice.is_empty() {
        summary.push_str(&format!(" \u{b7} {}", m.notice));
    }
    let playing_path = o.path.as_deref().filter(|_| loaded).map(|p| {
        let may = m.playing_for_agents.as_ref().is_some_and(|(q, ok)| q == p && *ok);
        seen::shown(p, agent, may)
    });
    View::new(summary)
        .with("track", title)
        .with("path", playing_path)
        .with("position", if loaded { json!(o.time_pos) } else { Value::Null })
        .with("duration", if loaded { json!(o.duration) } else { Value::Null })
        .with("playing", o.playing())
        .with("paused", loaded && o.paused)
        .with("volume", m.volume_now())
        .with("muted", m.muted_now())
        .with("queue_length", m.queue.len() as i64)
        .with("queue_position", m.queue.current_index().map(|i| i as i64 + 1))
        .with("library_size", m.library.len() as i64)
        .with("library", listed(m))
        .with("library_listed", m.shown.len() as i64)
        .with("folders", folders(m, agent))
        .with("scanning", scanning)
        .with("player", m.state.word())
        .with("player_problem", match &m.state {
            PlayerState::Unavailable(why) => Value::String(why.clone()),
            _ => Value::Null,
        })
        .with("status", m.agent_status.as_ref().filter(|_| agent).unwrap_or(&m.status).clone())
        .with("notice", m.notice.clone())
}

/// The first rows of the library list, numbered as `play`'s `index` takes them.
fn listed(m: &Model) -> Vec<Value> {
    m.shown
        .iter()
        .take(LISTED)
        .enumerate()
        .filter_map(|(n, &i)| m.library.get(i).map(|t| json!({"index": n + 1, "title": t.title, "folder": t.album})))
        .collect()
}

fn folders(m: &Model, agent: bool) -> Vec<Value> {
    m.folders
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let may = m.folders_for_agents.get(f).copied().unwrap_or(false);
            json!({"path": seen::shown(f, agent, may), "tracks": m.per_folder.get(i).copied().unwrap_or(0)})
        })
        .collect()
}
