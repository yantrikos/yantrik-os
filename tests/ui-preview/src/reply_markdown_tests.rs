//! A reply drawn as markdown (#1346), by the production Lens from fixture data: a paragraph with
//! bold, inline code and a link; headings by level; a numbered list whose long item wraps under
//! its text, with a nested bullet; a quote and a rule; a code block with a Copy button and a line
//! too long for the box; and a table too wide for it. The blocks are the ones `markdown_blocks.rs`
//! makes, written out by hand here because this runner is a separate workspace.
use super::chat_tests::{hermes_reply, named, save, settle, PANEL_X, W};
use super::*;
use slint::{ModelRc, StyledText, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

const LONG_LINE: &str = "    let report = build_report(&session, &options).with_title(\"A line long enough that it cannot fit the box\").render();";
const CODE: &str = "fn main() {\n    println!(\"hi\");\n\n}\n    let report = build_report(&session, &options).with_title(\"A line long enough that it cannot fit the box\").render();";

fn md(src: &str) -> StyledText {
    StyledText::from_markdown(src).expect("markdown the fixtures use")
}

fn block(kind: &str, text: &str, styled: StyledText) -> ContentBlock {
    ContentBlock { block_type: kind.into(), text: text.into(), styled, ..Default::default() }
}

fn item(marker: &str, depth: i32, src: &str) -> ContentBlock {
    ContentBlock { marker: marker.into(), depth, ..block("item", src, md(src)) }
}

fn table(rows: &[&[&str]]) -> ContentBlock {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(n, row)| TableRow {
            cells: ModelRc::new(VecModel::from(
                row.iter().map(|c| if n == 0 { md(&format!("**{c}**")) } else { md(c) }).collect::<Vec<_>>(),
            )),
        })
        .collect::<Vec<_>>();
    ContentBlock { rows: ModelRc::new(VecModel::from(rows)), ..block("table", "", StyledText::default()) }
}

fn reply() -> Vec<ContentBlock> {
    vec![
        ContentBlock { level: 1, ..block("heading", "Setting it up", StyledText::default()) },
        block("text", "", md("Run the **installer**, then check `config.toml`. The [guide](https://example.com/guide) has the rest.")),
        ContentBlock { level: 3, ..block("heading", "Steps", StyledText::default()) },
        item("1.", 0, "Open the **settings** pane and choose the account you want to change, then read the whole note on the page before you press anything at all"),
        item("2.", 0, "Press `Save`"),
        item("\u{25E6}", 1, "a nested point"),
        item("3.", 0, "Done"),
        block("quote", "", md("A note that someone **quoted** at length.")),
        block("rule", "", StyledText::default()),
        ContentBlock { lang: "rust".into(), ..block("code", CODE, StyledText::default()) },
        table(&[
            &["Name", "Role", "Where it lives", "Notes"],
            &["`alpha`", "**first**", "the north wing of the building", "a long cell that wraps onto a second line when it is narrow"],
            &["beta", "second", "south", "-"],
        ]),
        block("text", "", md("That is all.")),
    ]
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ChatProbe::new()?;
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, super::chat_tests::H));
    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let actions = ui.global::<ReplyActions>();
    actions.on_copy_text({
        let copied = copied.clone();
        let ui = ui.as_weak();
        move |text, id| {
            copied.borrow_mut().push(text.to_string());
            // Answer as the shell does once the copy is done.
            let actions = ui.unwrap().global::<ReplyActions>();
            actions.set_done_ok(true);
            actions.set_done_id(id);
        }
    });
    actions.on_open_link({
        let opened = opened.clone();
        move |url| opened.borrow_mut().push(url.to_string())
    });

    ui.set_messages(ModelRc::new(VecModel::from(vec![
        super::chat_tests::asking(),
        hermes_reply("Setting it up", false, reply()),
    ])));
    let shown = settle(w);
    save(&shown, &named(output, "reply-markdown"))?;

    // Copy: some press in the panel is the code block's Copy, and it hands on the block's exact
    // text — its indent, its blank line and its overlong line.
    'scan: for y in (90..780).step_by(6) {
        for x in (PANEL_X as i32 + 40..PANEL_X as i32 + 430).step_by(10) {
            crate::click(w, x as f32, y as f32);
            if !copied.borrow().is_empty() {
                break 'scan;
            }
        }
    }
    assert_eq!(copied.borrow().first().map(String::as_str), Some(CODE), "Copy hands on the code block's exact text");
    assert!(CODE.contains(LONG_LINE));

    // The link in the paragraph asks the shell to open it; the shell decides what is opened.
    'scan: for y in (90..780).step_by(4) {
        for x in (PANEL_X as i32 + 40..PANEL_X as i32 + 430).step_by(6) {
            crate::click(w, x as f32, y as f32);
            if !opened.borrow().is_empty() {
                break 'scan;
            }
        }
    }
    assert_eq!(opened.borrow().first().map(String::as_str), Some("https://example.com/guide"));
    Ok(())
}
