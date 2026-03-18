//! Yantrik Snippet Manager — standalone app binary.
//!
//! Manages code snippets with collections, tags, and search.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-snippet-manager");

    let app = SnippetManagerApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

// ── Default data ─────────────────────────────────────────────────────

fn default_collections() -> Vec<CollectionData> {
    vec![
        CollectionData {
            id: 0,
            name: "All Snippets".into(),
            snippet_count: 0,
            is_builtin: true,
            is_selected: true,
            icon: "\u{1F4CB}".into(),
        },
        CollectionData {
            id: 1,
            name: "Favorites".into(),
            snippet_count: 0,
            is_builtin: true,
            is_selected: false,
            icon: "\u{2B50}".into(),
        },
    ]
}

fn default_languages() -> Vec<LanguageOption> {
    ["Rust", "Python", "JavaScript", "TypeScript", "Go", "C", "C++", "Shell", "SQL", "HTML", "CSS", "TOML", "YAML", "JSON", "Other"]
        .iter()
        .map(|name| LanguageOption { name: (*name).into() })
        .collect()
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &SnippetManagerApp) {
    // Initialize with defaults
    app.set_snippets(ModelRc::new(VecModel::<SnippetData>::default()));
    app.set_collections(ModelRc::new(VecModel::from(default_collections())));
    app.set_language_options(ModelRc::new(VecModel::from(default_languages())));
    app.set_snippet_count(0);

    // Search
    app.on_snip_search(|query| {
        tracing::info!("Search snippets: {}", query);
    });

    // Select snippet
    {
        let weak = app.as_weak();
        app.on_snip_select(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_snippets();
            for i in 0..model.row_count() {
                if let Some(item) = model.row_data(i) {
                    if item.id == id {
                        ui.set_detail_id(item.id);
                        ui.set_detail_title(item.title.clone());
                        ui.set_detail_language(item.language.clone());
                        ui.set_detail_code(item.code.clone());
                        ui.set_detail_tags(item.tags.clone());
                        ui.set_detail_is_favorite(item.is_favorite);
                        ui.set_detail_created_date(item.created_date.clone());
                        ui.set_detail_last_used_date(item.last_used_date.clone());
                        ui.set_detail_use_count(item.use_count);
                        break;
                    }
                }
            }
        });
    }

    // New snippet
    {
        let weak = app.as_weak();
        app.on_snip_new(move || {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_snippets();
            let next_id = (0..model.row_count())
                .filter_map(|i| model.row_data(i).map(|s| s.id))
                .max()
                .unwrap_or(0)
                + 1;

            let new_snippet = SnippetData {
                id: next_id,
                title: "Untitled Snippet".into(),
                language: "Rust".into(),
                code: "// New snippet\n".into(),
                tags: "".into(),
                preview: "// New snippet".into(),
                date_text: "Just now".into(),
                is_selected: false,
                is_favorite: false,
                created_date: "Just now".into(),
                last_used_date: "".into(),
                use_count: 0,
                collection_id: 0,
            };

            let mut items: Vec<SnippetData> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            items.insert(0, new_snippet.clone());
            let count = items.len() as i32;
            ui.set_snippets(ModelRc::new(VecModel::from(items)));
            ui.set_snippet_count(count);

            // Select the new snippet
            ui.set_detail_id(new_snippet.id);
            ui.set_detail_title(new_snippet.title);
            ui.set_detail_language(new_snippet.language);
            ui.set_detail_code(new_snippet.code);
            ui.set_detail_tags(new_snippet.tags);
            ui.set_detail_is_favorite(false);
        });
    }

    // Save snippet
    app.on_snip_save(|id, title, language, code, tags| {
        tracing::info!("Save snippet {}: {} ({})", id, title, language);
        let _ = (code, tags); // suppress unused warnings
    });

    // Delete snippet
    {
        let weak = app.as_weak();
        app.on_snip_delete(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let model = ui.get_snippets();
            let remaining: Vec<SnippetData> = (0..model.row_count())
                .filter_map(|i| {
                    let item = model.row_data(i)?;
                    if item.id != id { Some(item) } else { None }
                })
                .collect();
            let count = remaining.len() as i32;
            ui.set_snippets(ModelRc::new(VecModel::from(remaining)));
            ui.set_snippet_count(count);
            ui.set_detail_id(-1);
        });
    }

    // Copy to clipboard
    app.on_snip_copy(|id| {
        tracing::info!("Copy snippet {} to clipboard", id);
    });

    // Tag filter
    app.on_snip_tag_filter(|tag| {
        tracing::info!("Filter by tag: {}", tag);
    });

    // Toggle favorite
    app.on_snip_toggle_favorite(|id| {
        tracing::info!("Toggle favorite for snippet {}", id);
    });

    // Collection select
    app.on_collection_select(|id| {
        tracing::info!("Select collection {}", id);
    });

    // Collection create
    app.on_collection_create(|name| {
        tracing::info!("Create collection: {}", name);
    });

    // Collection rename
    app.on_collection_rename(|id, name| {
        tracing::info!("Rename collection {} to {}", id, name);
    });

    // Collection delete
    app.on_collection_delete(|id| {
        tracing::info!("Delete collection {}", id);
    });

    // Export/Import
    app.on_snip_export_all(|| {
        tracing::info!("Export all snippets");
    });
    app.on_snip_import(|| {
        tracing::info!("Import snippets");
    });

    // Set language
    app.on_snip_set_language(|lang| {
        tracing::info!("Set language: {}", lang);
    });

    // Version history
    app.on_snip_view_version(|idx| {
        tracing::info!("View version {}", idx);
    });
    app.on_snip_restore_version(|idx| {
        tracing::info!("Restore version {}", idx);
    });

    // AI stubs
    app.on_ai_explain_pressed(|| {
        tracing::info!("AI explain requested (standalone mode)");
    });
    app.on_ai_dismiss(|| {});
}
