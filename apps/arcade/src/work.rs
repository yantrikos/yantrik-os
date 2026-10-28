//! The nine commands against the library, as the lane runs them. Each answer is built from its own
//! result, in the words the surface has always answered: a refusal is a sentence, never a code.
//!
//! Blocking: from the lane only, never from the UI thread.

use serde_json::{json, Value};

use crate::command::{Browse, Landing, Listing, Reply, Request, Verb};
use crate::library::Library;
use crate::spec::parse_game;

/// The whole of what the lane does for one command: run it, then read the library again, so the
/// listing that lands is the library as this command left it.
///
/// `running` is the browser jobs under way, as (action, game): a game with one is not rebuilt,
/// replaced or deleted under it.
pub fn run(library: &Library, seq: u64, request: Request, from_editor: bool, running: &[(String, String)]) -> Landing {
    let result = match busy_on(library, &request, running) {
        Some(sentence) => Err(sentence),
        None => command(library, &request),
    };
    Landing { seq, request: Some(request), from_editor, result, listing: list(library) }
}

/// The library read again, with no command.
pub fn reload(library: &Library, seq: u64) -> Landing {
    Landing { seq, request: None, from_editor: false, result: Ok(Reply::Reloaded), listing: list(library) }
}

/// A saved character's spec, for the editor.
pub fn character_spec(library: &Library, seq: u64, slug: &str) -> Landing {
    let result = library.load_character(slug).and_then(|spec| serde_json::to_string_pretty(&spec).map_err(|e| e.to_string()));
    Landing { seq, request: None, from_editor: false, result: result.map(Reply::Spec), listing: list(library) }
}

pub fn list(library: &Library) -> Listing {
    Listing { games: library.list_games(), characters: library.list_characters() }
}

/// Why `request` cannot run now: it would build, replace or delete a game a browser job is at
/// work on. The job would judge a file that is gone and write its verdict beside the new one.
fn busy_on(library: &Library, request: &Request, running: &[(String, String)]) -> Option<String> {
    if running.is_empty() {
        return None;
    }
    let key = match request.verb {
        Verb::Build | Verb::Delete => request.game.clone(),
        Verb::UpdateGame if !request.game.is_empty() => request.game.clone(),
        Verb::UpdateGame => parse_game(&request.spec).ok()?.title,
        _ => return None,
    };
    let (slug, _) = library.load_game_spec(&key).ok()?;
    let (action, _) = running.iter().find(|(_, subject)| *subject == slug)?;
    Some(format!("{action} of {slug} is running; wait for it to land (`describe` lists it under jobs), then try again"))
}

fn command(library: &Library, request: &Request) -> Result<Reply, String> {
    let done = |value: Value, select: Option<String>| Ok(Reply::Done { value, select, slow: None });
    match request.verb {
        Verb::NewCharacter => {
            let entry = library.save_character(&request.spec)?;
            done(
                json!({
                    "summary": format!("saved character {:?}", entry.name),
                    "saved": format!("saved character {:?}", entry.name),
                    "slug": entry.slug, "name": entry.name, "archetype": entry.archetype,
                }),
                None,
            )
        }
        Verb::NewGame => {
            let entry = library.save_game(&request.spec)?;
            done(
                json!({
                    "summary": format!("saved game {:?}; build it next", entry.title),
                    "saved": format!("saved game {:?}", entry.title),
                    "slug": entry.slug, "title": entry.title, "built": entry.built,
                }),
                Some(entry.slug),
            )
        }
        Verb::UpdateGame => {
            let existing = Some(request.game.as_str()).filter(|s| !s.is_empty());
            let entry = library.update_game(&request.spec, existing)?;
            done(
                json!({
                    "summary": format!("updated game {:?}; its old build is gone, build it next", entry.title),
                    "saved": format!("updated game {:?}", entry.title),
                    "slug": entry.slug, "title": entry.title, "built": entry.built,
                }),
                Some(entry.slug),
            )
        }
        Verb::UpdateCharacter => {
            let (entry, stale) = library.update_character(&request.spec)?;
            let summary = if stale.is_empty() {
                format!("updated character {:?}", entry.name)
            } else {
                format!(
                    "updated character {:?}; built with the old one and needing `build` again: {}",
                    entry.name,
                    stale.join(", ")
                )
            };
            done(
                json!({
                    "summary": summary,
                    "saved": format!("updated character {:?}", entry.name),
                    "slug": entry.slug, "name": entry.name, "archetype": entry.archetype,
                    "needs_build": stale,
                }),
                None,
            )
        }
        Verb::Build => {
            let (slug, path) = library.build_game(&request.game)?;
            done(
                json!({
                    "summary": format!("built {slug}"),
                    "built": format!("built {slug} \u{2192} {}", path.display()),
                    "slug": slug, "path": path.display().to_string(),
                }),
                Some(slug),
            )
        }
        Verb::Play | Verb::Verify | Verb::Screenshot => {
            // Resolve the game and check the build exists *now*, so an unknown or unbuilt game is
            // refused with its sentence; only real browser work goes to a worker.
            let (slug, _spec) = library.load_game_spec(&request.game)?;
            let html = library.game_html(&slug);
            if !html.exists() {
                return Err(format!("{slug} has not been built yet; `build game={slug}` first"));
            }
            Ok(Reply::Done { value: Value::Null, select: None, slow: Some(Browse { verb: request.verb, slug, html }) })
        }
        Verb::Delete => {
            let message = library.delete_game(&request.game)?;
            done(json!({ "summary": message.clone(), "deleted": message }), Some(String::new()))
        }
    }
}

/// What a started browser job answers: it is on a worker, and `describe` reports it when it lands.
pub fn started(verb: Verb, game: &str, job: u64) -> Value {
    json!({
        "summary": format!("{} of {game} started on a worker; `describe` reports it when it lands", verb.name()),
        "started": true,
        "job": job,
        "action": verb.name(),
        "game": game,
    })
}

/// One line out of a successful result, for the banner.
pub fn summary_of(value: &Value) -> String {
    for key in ["summary", "built", "saved", "deleted", "started"] {
        if let Some(text) = value.get(key).and_then(|v| v.as_str()) {
            return text.to_string();
        }
    }
    if value.get("started").is_some() {
        return "started".into();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_of_picks_the_human_line() {
        assert_eq!(summary_of(&json!({"summary": "built x"})), "built x");
        assert_eq!(summary_of(&json!({"built": "built x \u{2192} /p"})), "built x \u{2192} /p");
        assert_eq!(summary_of(&json!({"started": true})), "started");
    }

    #[test]
    fn a_missing_argument_is_named() {
        let err = Request::from_args(Verb::NewCharacter, &json!({})).unwrap_err();
        assert!(err.contains("`spec`"), "{err}");
        let err = Request::from_args(Verb::NewCharacter, &json!({"spec": "  "})).unwrap_err();
        assert!(err.contains("`spec`"), "{err}");
    }

    /// A call that names no game is refused, whatever the window has selected. The fallback was a
    /// default the caller could not see, and a grant binds the arguments as sent, so the card a
    /// person approved could run against a game nobody named, on `delete` above all.
    #[test]
    fn a_call_with_no_game_is_refused_rather_than_answered_from_the_window() {
        for verb in [Verb::Build, Verb::Play, Verb::Verify, Verb::Screenshot, Verb::Delete] {
            let err = Request::from_args(verb, &json!({})).unwrap_err();
            assert!(err.contains("`game` is required"), "{err}");
            let err = Request::from_args(verb, &json!({"game": "   "})).unwrap_err();
            assert!(err.contains("`game` is required"), "{err}");
        }
        assert_eq!(Request::from_args(Verb::Build, &json!({"game": " meadow-run "})).unwrap().game, "meadow-run");
        // update_game reads the game to replace off the spec's own title.
        assert_eq!(Request::from_args(Verb::UpdateGame, &json!({"spec": "{}"})).unwrap().game, "");
    }
}
