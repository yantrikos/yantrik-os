//! Skill Store wiring — populates UI model from SkillRegistry manifests.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

use yantrikdb_companion::skills::SkillRegistry;

use crate::app_context::AppContext;
use crate::App;

/// Wire the Skill Store callbacks and populate the initial skill list.
pub fn wire(ui: &App, ctx: &AppContext) {
    let registry = ctx.skill_registry.clone();

    // Populate initial skill list
    populate_skills(ui, &registry, "", "All");

    // Toggle skill callback
    let ui_weak = ui.as_weak();
    let reg2 = registry.clone();
    ui.on_toggle_skill(move |skill_id| {
        let id = skill_id.to_string();
        tracing::info!(skill = %id, "Skill toggle requested");

        // Open the skills.db to persist the change
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        let db_path = format!("{}/.config/yantrik/skills.db", home);
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            let mut reg = reg2.borrow_mut();
            let (new_state, auto_deps) = reg.toggle(&conn, &id);
            tracing::info!(skill = %id, enabled = new_state, auto_deps = ?auto_deps, "Skill toggled");
        }

        // Refresh UI
        if let Some(ui) = ui_weak.upgrade() {
            populate_skills(&ui, &reg2, "", "All");
        }
    });

    // Search callback
    let ui_weak = ui.as_weak();
    let reg3 = registry.clone();
    ui.on_search_skills(move |query| {
        let q = query.to_string();
        if let Some(ui) = ui_weak.upgrade() {
            populate_skills(&ui, &reg3, &q, "All");
        }
    });

    // Category filter callback
    let ui_weak = ui.as_weak();
    let reg4 = registry.clone();
    ui.on_filter_skill_category(move |category| {
        let cat = category.to_string();
        if let Some(ui) = ui_weak.upgrade() {
            populate_skills(&ui, &reg4, "", &cat);
        }
    });
}

/// Populate the skill list model with optional filtering.
fn populate_skills(
    ui: &App,
    registry: &Rc<RefCell<SkillRegistry>>,
    query: &str,
    category: &str,
) {
    let reg = registry.borrow();

    // Get filtered skills
    let entries = if !query.is_empty() {
        reg.search(query)
    } else if category != "All" {
        // Find matching category
        use yantrikdb_companion::skills::SkillCategory;
        let cat = match category.to_lowercase().as_str() {
            "productivity" => Some(SkillCategory::Productivity),
            "communication" => Some(SkillCategory::Communication),
            "development" => Some(SkillCategory::Development),
            "entertainment" => Some(SkillCategory::Entertainment),
            "smart home" => Some(SkillCategory::SmartHome),
            "finance" => Some(SkillCategory::Finance),
            "health" => Some(SkillCategory::Health),
            "news" => Some(SkillCategory::News),
            "system" => Some(SkillCategory::System),
            "search" => Some(SkillCategory::Search),
            "utility" => Some(SkillCategory::Utility),
            _ => None,
        };
        match cat {
            Some(c) => reg.filter_by_category(c),
            None => reg.list_all(),
        }
    } else {
        reg.list_all()
    };

    // Convert to Slint model
    let skill_items: Vec<crate::SkillData> = entries
        .iter()
        .map(|e| {
            let icon = icon_for_skill(&e.manifest.icon);
            crate::SkillData {
                id: SharedString::from(e.manifest.id.as_str()),
                name: SharedString::from(e.manifest.name.as_str()),
                description: SharedString::from(e.manifest.description.as_str()),
                icon: SharedString::from(icon),
                category: SharedString::from(e.manifest.category.as_str()),
                enabled: e.enabled,
                tags: SharedString::from(e.manifest.tags.join(", ").as_str()),
            }
        })
        .collect();

    let model = Rc::new(VecModel::from(skill_items));
    ui.set_skills(ModelRc::from(model));

    // Set categories
    let mut cats: Vec<SharedString> = vec![SharedString::from("All")];
    for cat in reg.active_categories() {
        cats.push(SharedString::from(cat.as_str()));
    }
    let cat_model = Rc::new(VecModel::from(cats));
    ui.set_skill_categories(ModelRc::from(cat_model));
}

/// Map skill icon name to emoji for display.
fn icon_for_skill(icon: &str) -> &str {
    match icon {
        "mail" => "\u{2709}\u{FE0F}",         // envelope
        "calendar" => "\u{1F4C5}",    // calendar
        "cloud" => "\u{26C5}",        // sun behind cloud
        "newspaper" => "\u{1F4F0}",   // newspaper
        "trending" => "\u{1F525}",    // fire
        "lightbulb" => "\u{1F4A1}",   // lightbulb
        "smile" => "\u{1F604}",       // grinning face
        "ticket" => "\u{1F4CB}",      // clipboard
        "git-branch" => "\u{1F500}",  // shuffle arrows (closest to git)
        "home" => "\u{1F3E0}",        // house
        "container" => "\u{1F4E6}",   // package
        "terminal" => "\u{1F4BB}",    // laptop
        "moon" => "\u{1F319}",        // crescent moon
        "shield" => "\u{1F6E1}\u{FE0F}",      // shield
        "bug" => "\u{1F41B}",         // bug
        "globe" => "\u{1F310}",       // globe
        "zap" => "\u{26A1}",          // lightning
        "clipboard" => "\u{1F4CB}",   // clipboard
        "package" => "\u{1F4E6}",     // package
        "puzzle" => "\u{1F9E9}",      // puzzle piece
        _ => "\u{1F9E9}",             // default: puzzle piece
    }
}
