//! The compiler: a resolved game spec in, one self-contained HTML file out.
//!
//! The compiler owns no game logic. It assembles three things that already exist —
//! the pinned vendored Three.js build, the engine, and the spec turned into the
//! engine's runtime JSON — into a single document with no network, no external
//! textures and nothing to install. Same spec in, byte-identical file out: there
//! are no timestamps and no randomness in the assembly, and the engine derives
//! every placement from a seed hashed out of the spec itself.

use serde_json::{json, Value};

use crate::spec::ResolvedGame;

/// The pinned Three.js build. r149, UMD, vendored in the repo with its MIT licence
/// beside it (vendor/THREE-LICENSE). Pinned by file, not by CDN: a built game must
/// still open in ten years with no network.
const THREE_JS: &str = include_str!("../vendor/three-r149.min.js");

/// The engine. Included from source so `cargo test` failures in the engine's
/// behaviour and the file a game embeds can never drift apart.
const ENGINE_JS: &str = include_str!("engine.js");

/// The character palette the grammar falls back to when a spec omits one.
/// In practice `validate` requires a palette, so this only guards the compiler
/// against a hand-built ResolvedGame.
fn default_palette() -> Value {
    json!({
        "base": "#44cc88",
        "belly": "#f2ead8",
        "accent": "#ff9f43",
        "nose": "#2f3640",
        "eye": "#2f3640"
    })
}

/// The runtime contract between compiler and engine: the exact object the engine
/// reads as `window.ARCADE_GAME`. Built from the *resolved* game, so a character
/// referenced by name is already inlined here — the engine never sees the library.
pub fn game_json(resolved: &ResolvedGame) -> Value {
    let spec = &resolved.spec;
    let character = &resolved.character;
    json!({
        "title": spec.title,
        "arena": {
            "size": spec.arena.size,
            "theme": spec.arena.theme,
        },
        "player": {
            "speed": spec.player.as_ref().map(|p| p.speed).unwrap_or(7.0),
        },
        "character": {
            // The name travels with the build: a saved character resolved by name
            // must still be recognisable inside the game file, and the engine's
            // HUD and win text can say who the player is.
            "name": character.name,
            "archetype": character.archetype,
            "proportions": character.proportions,
            "ears": character.ears,
            "tail": character.tail,
            "palette": character.palette.clone().map(|p| json!({
                "base": p.base, "belly": p.belly, "accent": p.accent,
                "nose": p.nose, "eye": p.eye,
            })).unwrap_or_else(default_palette),
            "expression": character.expression,
            "stance": character.stance,
        },
        "collectible": {
            "kind": spec.collectible.kind,
            "count": spec.collectible.count,
        },
        "hazards": spec.hazards.iter().map(|h| json!({
            "kind": h.kind, "speed": h.speed, "count": h.count,
        })).collect::<Vec<_>>(),
        "lives": spec.lives,
        "palette": spec.palette.as_ref().map(|p| json!({
            "ground": p.ground, "wall": p.wall, "sky": p.sky,
            "item": p.item, "hazard": p.hazard,
        })),
        "music": spec.music,
    })
}

/// Serialize JSON for embedding inside a `<script>` body. `<`, `>` and `&` are written as their
/// `\u00XX` escapes, which JSON reads back as the same characters. `</` alone would end the
/// script early, and `<!--<script>` would switch the parser into "double escaped" text where the
/// real `</script>` no longer closes the block; with no `<` left in the data, neither can be
/// made, whatever a title says.
fn json_for_script(value: &Value) -> String {
    serde_json::to_string(value)
        .expect("game_json builds valid JSON")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Compile a resolved game into its single HTML file.
pub fn compile(resolved: &ResolvedGame) -> String {
    let game = json_for_script(&game_json(resolved));
    // The engine is trusted repo content, but the same rule costs nothing and
    // keeps one policy for everything embedded.
    let engine = ENGINE_JS.replace("</", "<\\/");

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<!-- Built with Yantrik Arcade. Three.js r149 (MIT, licence in apps/arcade/vendor/THREE-LICENSE).
     One file, no network, no external assets. -->
<style>
  html, body {{ margin: 0; padding: 0; overflow: hidden; background: #101418; height: 100%; }}
  #game {{ display: block; width: 100vw; height: 100vh; }}
  #hud {{
    position: fixed; top: 12px; left: 14px; z-index: 2;
    font-family: ui-rounded, "Segoe UI", system-ui, sans-serif;
    color: #ffffff; text-shadow: 0 1px 3px rgba(0,0,0,.55);
    user-select: none; pointer-events: none;
  }}
  #hud-title {{ font-size: 15px; font-weight: 700; letter-spacing: .04em; opacity: .92; }}
  #hud-score {{ font-size: 26px; font-weight: 800; line-height: 1.15; }}
  #hud-score.pop {{ animation: hudpop .28s ease-out; }}
  @keyframes hudpop {{ 0% {{ transform: scale(1); }} 40% {{ transform: scale(1.28); }} 100% {{ transform: scale(1); }} }}
  #hud-lives {{ font-size: 18px; letter-spacing: .12em; color: #ff8a95; }}
  #overlay {{
    position: fixed; inset: 0; z-index: 3; display: none;
    align-items: center; justify-content: center; flex-direction: column;
    background: rgba(10, 12, 16, .55); backdrop-filter: blur(2px);
    font-family: ui-rounded, "Segoe UI", system-ui, sans-serif; color: #fff;
    text-align: center; user-select: none;
  }}
  #overlay-text {{ font-size: 44px; font-weight: 900; letter-spacing: .06em; text-shadow: 0 2px 10px rgba(0,0,0,.6); }}
  #overlay-hint {{ margin-top: 10px; font-size: 15px; opacity: .85; max-width: 70vw; }}
</style>
</head>
<body>
<canvas id="game"></canvas>
<div id="hud">
  <div id="hud-title"></div>
  <div id="hud-score"></div>
  <div id="hud-lives"></div>
</div>
<div id="overlay">
  <div id="overlay-text"></div>
  <div id="overlay-hint"></div>
</div>
<script>
// Three.js r149 (pinned, vendored, MIT). Classic script, so it lands as the global THREE.
{three}
</script>
<script>
window.ARCADE_GAME = {game};
</script>
<script>
{engine}
</script>
</body>
</html>
"##,
        title = html_escape_title(&resolved.spec.title),
        three = THREE_JS,
        game = game,
        engine = engine,
    )
}

/// The title also lands in the document's `<title>`, which is ordinary HTML —
/// the JSON escaping above does not cover it.
fn html_escape_title(title: &str) -> String {
    title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::parse_game;

    fn resolved(title: &str) -> ResolvedGame {
        let spec_text = format!(
            r##"{{
                "title": {title},
                "player": {{ "character": {{
                    "name": "Pip",
                    "archetype": "critter",
                    "palette": {{ "base": "#44cc88", "belly": "#f2ead8", "accent": "#ff9f43", "nose": "#2f3640", "eye": "#2f3640" }}
                }} }},
                "collectible": {{ "kind": "berry", "count": 5 }},
                "hazards": [{{ "kind": "wanderer", "speed": 2.0, "count": 2 }}]
            }}"##
        );
        let spec = parse_game(&spec_text).expect("test spec must parse");
        ResolvedGame::resolve(spec, &|_| Err("no library in tests".into())).expect("inline character resolves")
    }

    #[test]
    fn compiles_to_one_self_contained_document() {
        let html = compile(&resolved("\"Pip's Meadow Run\""));
        assert!(html.starts_with("<!doctype html>"));
        // Three <script> blocks, all inline: no src=, no link=, nothing fetched.
        assert_eq!(html.matches("<script>").count(), 3);
        assert_eq!(html.matches("</script>").count(), 3);
        assert!(!html.contains("<script src"));
        assert!(!html.contains("<link"));
    }

    #[test]
    fn the_engine_and_the_runtime_json_are_both_in_there() {
        let html = compile(&resolved("\"Pip's Meadow Run\""));
        assert!(html.contains("window.ARCADE_GAME = {"));
        assert!(html.contains("window.__arcade"));
        assert!(html.contains("CapsuleGeometry"), "the pinned Three build must be the one vendored");
    }

    #[test]
    fn a_script_tag_in_the_title_cannot_escape_the_json() {
        let html = compile(&resolved("\"</script><b>boo</b>\""));
        // Still exactly three closers: the hostile title is inert data.
        assert_eq!(html.matches("</script>").count(), 3);
        assert!(html.contains("\\u003c/script\\u003e\\u003cb\\u003eboo"));
        // And the HTML title element escaped it independently.
        assert!(html.contains("<title>&lt;/script&gt;&lt;b&gt;boo&lt;/b&gt;</title>"));
    }

    /// `<!--<script>` in the data puts the HTML parser in "script data double escaped", where the
    /// real `</script>` no longer closes the block and the game does not boot.
    #[test]
    fn a_comment_and_script_opener_in_the_title_cannot_unbalance_the_script() {
        let html = compile(&resolved("\"<!--<script>&amp;\""));
        assert_eq!(html.matches("</script>").count(), 3, "the real closers still close");
        assert!(!html.contains("<!--<script>"), "the opener never appears in the data");
        assert!(html.contains("\\u003c!--\\u003cscript\\u003e\\u0026amp;"));
    }

    #[test]
    fn the_same_spec_compiles_byte_identical() {
        let a = compile(&resolved("\"Twin Test\""));
        let b = compile(&resolved("\"Twin Test\""));
        assert_eq!(a, b, "compiling must be deterministic: no timestamps, no randomness");
    }

    #[test]
    fn a_different_title_compiles_differently() {
        let a = compile(&resolved("\"Game A\""));
        let b = compile(&resolved("\"Game B\""));
        assert_ne!(a, b);
    }

    #[test]
    fn the_runtime_json_carries_the_resolved_contract() {
        let g = game_json(&resolved("\"Contract Check\""));
        assert_eq!(g["title"], "Contract Check");
        assert_eq!(g["arena"]["size"], 18.0); // grammar default
        assert_eq!(g["arena"]["theme"], "meadow");
        assert_eq!(g["player"]["speed"], 7.0);
        assert_eq!(g["character"]["name"], "Pip");
        assert_eq!(g["character"]["archetype"], "critter");
        assert_eq!(g["character"]["proportions"]["head_body"], 1.0);
        assert_eq!(g["character"]["palette"]["base"], "#44cc88");
        assert_eq!(g["collectible"], json!({ "kind": "berry", "count": 5 }));
        assert_eq!(g["hazards"][0], json!({ "kind": "wanderer", "speed": 2.0, "count": 2 }));
        assert_eq!(g["lives"], 3);
        assert!(g["palette"].is_null(), "no arena palette in this spec");
        assert_eq!(g["music"], "bouncy");
    }

    #[test]
    fn json_for_script_never_emits_a_raw_closer() {
        let v = json!({ "title": "</script></SCRIPT></x" });
        let s = json_for_script(&v);
        assert!(!s.contains("</"));
        // The escape still parses back to the original string.
        let back: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
    }
}
