//! A deck is untrusted content: its text reaches the window and `describe` as plain strings, and
//! nothing in it is interpreted as markup, a path or a command. A hostile file is refused whole.

use serde_json::json;
use slint::Model as _;

use super::{act, deck_folder, notice, outline, scratch, started, state, summary};

/// Markup of every kind a surface might read as something other than text.
const HOSTILE: &str = "<b>bold</b> {x} @image-url(\"/etc/passwd\") \\u{202e} <script>alert(1)</script> \
                       [click](file:///etc/passwd) `rm -rf ~` $(touch /tmp/ypresent-pwned) ${HOME}";

#[test]
fn a_slides_markup_is_drawn_and_reported_as_the_text_it_is() {
    let app = started();
    act(&app, "set_slide", json!({ "index": 0, "title": HOSTILE, "body": HOSTILE, "notes": HOSTILE })).unwrap();
    assert_eq!(app.ui(|ui| ui.get_current_title().to_string()), HOSTILE, "the canvas holds exactly the text");
    assert_eq!(app.ui(|ui| ui.get_slides().row_data(0).unwrap().body.to_string()), HOSTILE);
    assert_eq!(state(&app)["current"]["title"], HOSTILE);
    assert_eq!(outline(&app), vec![HOSTILE]);
    let md = app.model(|m| m.deck().to_markdown());
    assert!(md.contains(&format!("# {HOSTILE}\n")), "an export carries it as text too");
    assert!(!std::path::Path::new("/tmp/ypresent-pwned").exists(), "nothing in a deck is run");
}

#[test]
fn a_decks_name_in_a_notice_a_title_or_a_refusal_is_just_text() {
    let app = started();
    deck_folder(&app);
    act(&app, "new_deck", json!({ "title": "<i>ital $(x)" })).unwrap();
    assert_eq!(app.ui(|ui| ui.get_presentation_title().to_string()), "<i>ital $(x)");
    assert!(summary(&app).contains("<i>ital $(x)"), "{}", summary(&app));
    let dir = scratch("hostile-names");
    let odd = dir.join("<b>deck $(x).ydeck");
    act(&app, "save_as", json!({ "path": odd.display().to_string() })).unwrap();
    assert_eq!(state(&app)["file"], json!(odd.display().to_string()));
    let err = act(&app, "open", json!({ "path": dir.join("<u>missing</u>.ydeck").display().to_string() })).unwrap_err();
    assert_eq!(notice(&app), err, "a refusal that names a hostile path says it as text");
}

#[test]
fn a_hostile_file_is_refused_whole_and_costs_nothing() {
    let app = started();
    let dir = scratch("hostile-files");
    let nested = dir.join("nested.ydeck");
    std::fs::write(&nested, format!("{}{}", "[".repeat(100_000), "]".repeat(100_000))).unwrap();
    let err = act(&app, "open", json!({ "path": nested.display().to_string() })).unwrap_err();
    assert!(err.contains("it is not a yPresent deck"), "deeply nested JSON is refused, not recursed into: {err}");
    let many = dir.join("many.ydeck");
    let slides = vec![json!({ "title": "s", "body": "", "notes": "", "layout": 0 }); crate::deck::MAX_SLIDES + 1];
    std::fs::write(&many, json!({ "version": 1, "title": "t", "slides": slides }).to_string()).unwrap();
    let err = act(&app, "open", json!({ "path": many.display().to_string() })).unwrap_err();
    assert!(err.contains("over the 500 this build will open"), "{err}");
    let named = dir.join("../../../etc/hostname");
    assert!(act(&app, "open", json!({ "path": named.display().to_string() })).is_err());
    assert_eq!(outline(&app), vec!["Title Slide"]);
}
