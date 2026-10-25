//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that writes the window.

use slint::{ComponentHandle as _, ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::rail::{self, RailView as _};

use crate::command::Command;
use crate::model::{Editor, Model, PART_COPY, PART_DETAIL, PART_LIST, PART_NOTICE, PART_RAISE, PART_RECOVERY};
use crate::update::drafts;
use crate::store::{text, ALL, FAVORITES};
use crate::{CollectionData, LanguageOption, SnippetData, SnippetManager, SnippetManagerApp, TagChip};

/// The language menu's choices.
const LANGUAGES: [&str; 15] = [
    "Rust", "Python", "JavaScript", "TypeScript", "Go", "C", "C++", "Shell", "SQL", "HTML", "CSS",
    "TOML", "YAML", "JSON", "Other",
];

pub fn wire(cx: &Cx<SnippetManager>) {
    let ui = cx.ui();
    let languages: Vec<LanguageOption> = LANGUAGES.iter().map(|name| LanguageOption { name: (*name).into() }).collect();
    ui.set_language_options(ModelRc::new(VecModel::from(languages)));

    ui.on_snip_search(cx.cmd1(|query: SharedString| Command::Search(query.to_string())));
    ui.on_snip_tag_filter(cx.cmd1(|tag: SharedString| Command::TagFilter(tag.to_string())));
    ui.on_snip_select(cx.cmd1(Command::Select));
    ui.on_snip_new(cx.cmd(|| Command::New));
    ui.on_snip_delete(cx.cmd1(Command::Delete));
    ui.on_snip_copy(cx.cmd1(Command::Copy));
    ui.on_snip_toggle_favorite(cx.cmd1(Command::ToggleFavorite));
    ui.on_snip_set_language(cx.cmd1(|language: SharedString| Command::SetLanguage(language.to_string())));
    ui.on_collection_select(cx.cmd1(Command::CollectionSelect));
    ui.on_collection_create(cx.cmd1(|name: SharedString| Command::CollectionCreate(name.to_string())));
    ui.on_collection_rename(cx.cmd2(|id: i32, name: SharedString| Command::CollectionRename(id, name.to_string())));
    ui.on_collection_delete(cx.cmd1(Command::CollectionDelete));
    ui.on_snip_export_all(cx.cmd(|| Command::ExportAll));
    ui.on_ai_explain_pressed(cx.cmd(|| Command::AiExplain));
    ui.on_agent_context_activated(cx.cmd1(|id: SharedString| Command::ContextActivated(id.to_string())));
    ui.on_recovery_action(cx.try_cmd1(|action: SharedString| drafts::choice(&action)));
    // Save sends the four fields as the editor has them.
    let saving = cx.clone();
    ui.on_snip_save(move |id, title, language, code, tags| {
        saving.run(Command::Save(Editor {
            id,
            title: title.to_string(),
            language: language.to_string(),
            code: code.to_string(),
            tags: tags.to_string(),
        }));
    });
    // The editor's typing, as the person types it: the model holds it, so a close, a logout and
    // the draft lane all have it.
    let typed = cx.clone();
    ui.on_detail_edited(move || {
        let editor = editor_of(&typed.ui());
        typed.run(Command::Edited(editor));
    });
    drop(ui);
    rail::wire(cx);
}

/// The editor's fields as the window has them now.
fn editor_of(ui: &SnippetManagerApp) -> Editor {
    Editor {
        id: ui.get_detail_id(),
        title: ui.get_detail_title().to_string(),
        language: ui.get_detail_language().to_string(),
        code: ui.get_detail_code().to_string(),
        tags: ui.get_detail_tags().to_string(),
    }
}

pub fn render(ui: &SnippetManagerApp, m: &Model, changed: &Changed) {
    if changed.has(PART_LIST) {
        list(ui, m);
    }
    if changed.has(PART_DETAIL) {
        detail(ui, m);
    }
    // Whether the offer's snippet is open is the editor's to say, so the bar follows it too.
    if changed.has(PART_RECOVERY) || changed.has(PART_DETAIL) {
        let (said, state) = drafts::bar(m).unwrap_or_default();
        ui.set_recovery_said(said.into());
        ui.set_recovery(state);
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_COPY) {
        ui.set_copy_feedback_visible(m.copied);
    }
    if changed.has(PART_RAISE) {
        ui.window().set_minimized(false);
    }
    if changed.has(rail::PART) {
        ui.render_rail(&m.rail);
    }
}

/// The list, the chips, the sidebar and the counts. Never the editor: typing in the search box
/// repaints the list on every keystroke, and a repaint that also rewrote the editor would throw
/// away whatever was being typed into it.
fn list(ui: &SnippetManagerApp, m: &Model) {
    let now = text::now();
    let rows: Vec<SnippetData> = m
        .rows()
        .iter()
        .map(|s| SnippetData {
            id: s.id,
            title: s.title.as_str().into(),
            language: s.language.as_str().into(),
            code: s.code.as_str().into(),
            tags: s.tags.as_str().into(),
            preview: s.preview().into(),
            date_text: text::relative(s.updated, now).into(),
            is_selected: s.id == m.selected,
            is_favorite: s.favorite,
            created_date: text::date(s.created).into(),
            last_used_date: text::relative(s.used, now).into(),
            use_count: s.use_count,
            collection_id: s.collection,
        })
        .collect();
    let matched = rows.len();
    ui.set_snippets(ModelRc::new(VecModel::from(rows)));

    let chips: Vec<TagChip> = m
        .library
        .tags()
        .into_iter()
        .map(|name| TagChip { is_active: name.to_lowercase() == m.tag.to_lowercase(), name: name.into() })
        .collect();
    ui.set_tag_chips(ModelRc::new(VecModel::from(chips)));

    let total = m.library.snippets().len();
    let mut collections = vec![
        CollectionData {
            id: ALL,
            name: "All Snippets".into(),
            snippet_count: total as i32,
            is_builtin: true,
            is_selected: m.collection == ALL,
            icon: "\u{1F4CB}".into(),
        },
        CollectionData {
            id: FAVORITES,
            name: "Favorites".into(),
            snippet_count: m.library.favorites() as i32,
            is_builtin: true,
            is_selected: m.collection == FAVORITES,
            icon: "\u{2B50}".into(),
        },
    ];
    for collection in m.library.collections() {
        collections.push(CollectionData {
            id: collection.id,
            name: collection.name.as_str().into(),
            snippet_count: m.library.snippets().iter().filter(|s| s.collection == collection.id).count() as i32,
            is_builtin: false,
            is_selected: m.collection == collection.id,
            icon: "\u{1F4C1}".into(),
        });
    }
    ui.set_collections(ModelRc::new(VecModel::from(collections)));

    ui.set_snippet_count(total as i32);
    // -1 is the screen's "no search is narrowing this", which is not the same as "nothing
    // matched" — and drawing 0 for both was how the old blank window looked deliberate.
    ui.set_match_count(if m.query.trim().is_empty() && m.tag.is_empty() { -1 } else { matched as i32 });
    // Set only where it differs: the box it was typed in already shows it, and its caret stays.
    if ui.get_search_query().as_str() != m.query {
        ui.set_search_query(m.query.as_str().into());
    }
}

/// The editor pane: its fields from the editor, where they differ from what the window shows (so
/// the field being typed in keeps its caret), and the rest from what is kept.
fn detail(ui: &SnippetManagerApp, m: &Model) {
    let e = &m.editor;
    ui.set_detail_id(e.id);
    set_if_differs(ui.get_detail_title(), &e.title, |v| ui.set_detail_title(v));
    set_if_differs(ui.get_detail_language(), &e.language, |v| ui.set_detail_language(v));
    set_if_differs(ui.get_detail_code(), &e.code, |v| ui.set_detail_code(v));
    set_if_differs(ui.get_detail_tags(), &e.tags, |v| ui.set_detail_tags(v));
    let open = m.open();
    let now = text::now();
    ui.set_detail_is_favorite(open.is_some_and(|s| s.favorite));
    ui.set_detail_created_date(open.map(|s| text::date(s.created)).unwrap_or_default().into());
    ui.set_detail_last_used_date(open.map(|s| text::relative(s.used, now)).unwrap_or_default().into());
    ui.set_detail_use_count(open.map_or(0, |s| s.use_count));
}

fn set_if_differs(shown: SharedString, wanted: &str, set: impl FnOnce(SharedString)) {
    if shown.as_str() != wanted {
        set(wanted.into());
    }
}
