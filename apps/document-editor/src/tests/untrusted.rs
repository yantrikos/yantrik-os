//! A document is untrusted content: its text reaches the window and `describe` as plain strings,
//! and nothing in it is interpreted as markup, a path or a command.

use serde_json::json;
use slint::Model as _;

use super::{act, scratch, started, state, summary};

/// Markup of every kind a surface might read as something other than text.
const HOSTILE: &str = "# <b>bold</b> {x} @image-url(\"/etc/passwd\") \\u{202e}\n\n<script>alert(1)</script> \
                       [click](file:///etc/passwd) `rm -rf ~` $(touch /tmp/pwned) ${HOME}\n";

#[test]
fn a_documents_markup_is_drawn_and_reported_as_the_text_it_is() {
    let app = started();
    act(&app, "set_content", json!({ "text": HOSTILE })).unwrap();
    assert_eq!(app.ui(|ui| ui.get_doc_content().to_string()), HOSTILE, "the page holds exactly the text");
    let s = state(&app);
    assert_eq!(s["content"], HOSTILE);
    assert_eq!(s["title"], "<b>bold</b> {x} @image-url(\"/etc/passwd\") \\u{202e}", "a title is text, not a tag");
    assert_eq!(app.ui(|ui| ui.get_doc_title().to_string()), s["title"].as_str().unwrap());
    assert!(summary(&app).contains("<b>bold</b>"), "{}", summary(&app));
    let outline = app.ui(|ui| ui.get_doc_headings().row_data(0).unwrap().title.to_string());
    assert_eq!(outline, s["title"].as_str().unwrap());
    assert!(!std::path::Path::new("/tmp/pwned").exists(), "nothing in a document is run");
}

#[test]
fn a_documents_text_in_a_notice_or_a_refusal_is_just_text() {
    let app = started();
    let dir = scratch("hostile-names");
    let odd = dir.join("<i>ital $(x).md");
    act(&app, "set_content", json!({ "text": "body\n" })).unwrap();
    act(&app, "save_as", json!({ "path": odd.display().to_string() })).unwrap();
    assert_eq!(state(&app)["title"], "<i>ital $(x).md");
    let refused = act(&app, "save", json!({})).ok();
    assert!(refused.is_some(), "a clean document saves again");
    // The html export escapes the title it puts in the page.
    let html = crate::markdown::to_html("# t\n", "<script>x</script> & y");
    assert!(html.contains("<title>&lt;script&gt;x&lt;/script&gt; &amp; y</title>"), "{html}");
}
