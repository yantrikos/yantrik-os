//! The window: its callbacks bound to commands, and how it is drawn (render.rs).

pub mod render;

use slint::{ComponentHandle as _, SharedString};
use yantrik_app::cx::Cx;

use crate::command::{Command, WindowAction};
use crate::markdown::Format;
use crate::{DocumentEditorApp, YDoc};

/// Binds the window's callbacks. Where a callback carries no value, the window's own fields are
/// read here and the command carries them: the model is told what the window shows, and never
/// reads the window itself.
pub fn wire(cx: &Cx<YDoc>, ui: &DocumentEditorApp) {
    wire_document(cx, ui);
    wire_formatting(cx, ui);
    wire_find(cx, ui);
    wire_dialog(cx, ui);
    wire_agent(cx, ui);
}

/// How one button's `on_*` is registered: the callback to bind, as the window's setter takes it.
type Register<'a> = &'a dyn Fn(&DocumentEditorApp, Box<dyn Fn()>);

/// The selection the window holds, as byte offsets into the page.
fn selection(ui: &DocumentEditorApp) -> (usize, usize) {
    (ui.get_doc_anchor().max(0) as usize, ui.get_doc_cursor().max(0) as usize)
}

fn wire_document(cx: &Cx<YDoc>, ui: &DocumentEditorApp) {
    ui.on_doc_new(cx.cmd(|| Command::Window(WindowAction::New)));
    ui.on_doc_open(cx.cmd(|| Command::Window(WindowAction::Open)));
    ui.on_doc_save(cx.cmd(|| Command::Window(WindowAction::Save)));
    ui.on_doc_save_as(cx.cmd(|| Command::Window(WindowAction::SaveAs)));
    ui.on_doc_export_md(cx.cmd(|| Command::Window(WindowAction::ExportMarkdown)));
    ui.on_doc_export_html(cx.cmd(|| Command::Window(WindowAction::ExportHtml)));
    ui.on_doc_content_changed(cx.cmd1(|text: SharedString| Command::Edit(text.to_string())));
    ui.on_doc_heading_clicked(cx.cmd1(|offset: i32| Command::Goto(offset.max(0) as usize)));

    // The page keeps its own edit history under the caret. These press it and then take the text
    // back out, because Slint's undo does not raise `edited` and the document would otherwise be
    // one step behind what is on screen.
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_undo(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_undo_body();
            run.run(Command::Edit(ui.get_doc_content().to_string()));
        }
    });
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_redo(move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_redo_body();
            run.run(Command::Edit(ui.get_doc_content().to_string()));
        }
    });
}

/// One formatting button. They differ only in which callback and which mark, and writing eleven
/// near-identical blocks is how one of them ends up wired to the wrong thing.
fn wire_formatting(cx: &Cx<YDoc>, ui: &DocumentEditorApp) {
    let button = |register: Register<'_>, what: Format| {
        let (run, weak) = (cx.clone(), ui.as_weak());
        register(
            ui,
            Box::new(move || {
                if let Some(ui) = weak.upgrade() {
                    let (anchor, cursor) = selection(&ui);
                    run.run(Command::Format { what, anchor, cursor });
                }
            }),
        );
    };
    button(&|u, f| u.on_doc_format_bold(f), Format::Bold);
    button(&|u, f| u.on_doc_format_italic(f), Format::Italic);
    button(&|u, f| u.on_doc_format_strikethrough(f), Format::Strikethrough);
    button(&|u, f| u.on_doc_format_bullet(f), Format::Bullet);
    button(&|u, f| u.on_doc_format_checklist(f), Format::Checklist);
    button(&|u, f| u.on_doc_format_quote(f), Format::Quote);
    button(&|u, f| u.on_doc_format_code(f), Format::CodeBlock);
    button(&|u, f| u.on_doc_format_inline_code(f), Format::InlineCode);
    button(&|u, f| u.on_doc_format_divider(f), Format::Divider);

    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_format_heading(move |level: i32| {
        if let Some(ui) = weak.upgrade() {
            let (anchor, cursor) = selection(&ui);
            run.run(Command::Format { what: Format::Heading(level.clamp(1, 6) as u8), anchor, cursor });
        }
    });
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_format_link(move |url: SharedString| {
        if let Some(ui) = weak.upgrade() {
            let (anchor, cursor) = selection(&ui);
            run.run(Command::Link { url: url.to_string(), anchor, cursor });
        }
    });
}

fn wire_find(cx: &Cx<YDoc>, ui: &DocumentEditorApp) {
    ui.on_doc_find_changed(cx.cmd1(|query: SharedString| Command::FindChanged(query.to_string())));
    ui.on_doc_find_next(cx.cmd(|| Command::Window(WindowAction::FindNext)));
    ui.on_doc_find_prev(cx.cmd(|| Command::Window(WindowAction::FindPrev)));
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_replace_one(move || {
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Window(WindowAction::ReplaceOne { with: ui.get_doc_replace_text().to_string() }));
        }
    });
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_replace_all(move || {
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Window(WindowAction::ReplaceAll { with: ui.get_doc_replace_text().to_string() }));
        }
    });
}

fn wire_dialog(cx: &Cx<YDoc>, ui: &DocumentEditorApp) {
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_doc_dialog_confirm(move || {
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Window(WindowAction::Confirm { path: ui.get_doc_dialog_path().to_string() }));
        }
    });
    ui.on_doc_dialog_cancel(cx.cmd(|| Command::Window(WindowAction::Cancel)));
    ui.on_doc_dialog_discard(cx.cmd(|| Command::Window(WindowAction::Discard)));
}

/// The AI panel's three controls ask through the rail's path; a heading in the rail goes to it.
fn wire_agent(cx: &Cx<YDoc>, ui: &DocumentEditorApp) {
    yantrik_app::rail::wire(cx);
    ui.on_doc_ai_submit(cx.cmd1(|prompt: SharedString| Command::Ask(format!("{}{prompt}", crate::rail::ASK))));
    ui.on_doc_ai_summarize(cx.cmd(|| Command::Ask(crate::rail::SUMMARIZE.into())));
    ui.on_doc_ai_improve(cx.cmd(|| Command::Ask(crate::rail::TIGHTEN.into())));
    // The outline rows in the rail carry the same byte offset the sidebar does, so pressing one
    // goes to that heading. A memory row has nowhere in this document to go.
    let run = cx.clone();
    ui.on_agent_context_activated(move |id: SharedString| {
        if let Some(offset) = id.strip_prefix("heading:").and_then(|o| o.parse::<usize>().ok()) {
            run.run(Command::Goto(offset));
        }
    });
}
