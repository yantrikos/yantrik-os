//! Command Palette wiring — Ctrl+Shift+P overlay with fuzzy command search.
//!
//! Builds a static command registry at startup from all apps. Provides fuzzy
//! filtering and action resolution when a command is selected.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app_context::AppContext;
use crate::App;

// Re-export the Slint struct
use crate::CommandItem;

/// Wire command palette callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    wire_search(ui);
    wire_selected(ui, ctx);
    wire_open(ui);
}

/// Build the full command registry.
fn build_commands() -> Vec<CommandItem> {
    let mut cmds = Vec::with_capacity(80);

    // ── Navigation ──
    nav(&mut cmds, "Go to Desktop", "1", "⌂", "nav:1");
    nav(&mut cmds, "Go to Files", "8", "F", "nav:8");
    nav(&mut cmds, "Go to Settings", "7", "⚙", "nav:7");
    nav(&mut cmds, "Go to Terminal", "14", ">_", "nav:14");
    nav(&mut cmds, "Go to Notes", "15", "✎", "nav:15");
    nav(&mut cmds, "Go to Email", "17", "@", "nav:17");
    nav(&mut cmds, "Go to Calendar", "18", "▦", "nav:18");
    nav(&mut cmds, "Go to Weather", "19", "W", "nav:19");
    nav(&mut cmds, "Go to Music", "20", "♪", "nav:20");
    nav(&mut cmds, "Go to Packages", "21", "P", "nav:21");
    nav(&mut cmds, "Go to Network", "22", "N", "nav:22");
    nav(&mut cmds, "Go to System Monitor", "23", "◉", "nav:23");
    nav(&mut cmds, "Go to Image Viewer", "11", "I", "nav:11");
    nav(&mut cmds, "Go to Text Editor", "12", "≡", "nav:12");
    nav(&mut cmds, "Go to Media Player", "13", "▶", "nav:13");
    nav(&mut cmds, "Go to Spreadsheet", "29", "YS", "nav:29");
    nav(&mut cmds, "Go to Document Editor", "30", "YD", "nav:30");
    nav(&mut cmds, "Go to Presentation", "31", "YP", "nav:31");

    // ── Email ──
    cmd(&mut cmds, "Compose New Email", "Email", "@", "email:compose", "");
    cmd(&mut cmds, "Check Mail", "Email", "@", "email:check", "");
    cmd(&mut cmds, "Search Email", "Email", "@", "email:search", "");

    // ── Calendar ──
    cmd(&mut cmds, "New Calendar Event", "Calendar", "▦", "calendar:new", "");
    cmd(&mut cmds, "Today's Events", "Calendar", "▦", "calendar:today", "");

    // ── Notes ──
    cmd(&mut cmds, "New Note", "Notes", "✎", "notes:new", "");
    cmd(&mut cmds, "Search Notes", "Notes", "✎", "notes:search", "");

    // ── Terminal ──
    cmd(&mut cmds, "New Terminal Tab", "Terminal", ">_", "terminal:new-tab", "Ctrl+T");

    // ── Text Editor ──
    cmd(&mut cmds, "New Editor Tab", "Editor", "≡", "editor:new-tab", "");
    cmd(&mut cmds, "Open File in Editor", "Editor", "≡", "editor:open", "");

    // ── Spreadsheet ──
    cmd(&mut cmds, "New Spreadsheet", "Sheets", "YS", "spreadsheet:new", "");
    cmd(&mut cmds, "Import CSV", "Sheets", "YS", "spreadsheet:import-csv", "");

    // ── Document ──
    cmd(&mut cmds, "New Document", "Document", "YD", "document:new", "");

    // ── Presentation ──
    cmd(&mut cmds, "New Presentation", "Slides", "YP", "presentation:new", "");

    // ── System ──
    cmd(&mut cmds, "Lock Screen", "System", "L", "system:lock", "");
    cmd(&mut cmds, "Take Screenshot", "System", "S", "system:screenshot", "");
    cmd(&mut cmds, "Toggle Dark Mode", "System", "◐", "system:toggle-theme", "");
    cmd(&mut cmds, "Open Quick Settings", "System", "⚙", "system:quick-settings", "");
    cmd(&mut cmds, "Toggle DND Mode", "System", "D", "system:toggle-dnd", "");
    cmd(&mut cmds, "View Notifications", "System", "N", "system:notifications", "");
    cmd(&mut cmds, "About Yantrik OS", "System", "i", "system:about", "");
    cmd(&mut cmds, "Power Menu", "System", "⏻", "system:power", "");

    // ── AI ──
    cmd(&mut cmds, "Ask AI", "AI", "◈", "ai:ask", "");
    cmd(&mut cmds, "Summarize Current", "AI", "◈", "ai:summarize", "");
    cmd(&mut cmds, "AI Morning Brief", "AI", "◈", "ai:morning-brief", "");

    // ── Search ──
    cmd(&mut cmds, "Global Search", "Search", "?", "search:global", "");
    cmd(&mut cmds, "Search Files", "Search", "F", "search:files", "");
    cmd(&mut cmds, "Search Memory", "Search", "◈", "search:memory", "");

    cmds
}

/// Filter commands by query (fuzzy case-insensitive match on label + category).
fn filter_commands(commands: &[CommandItem], query: &str) -> Vec<CommandItem> {
    if query.is_empty() {
        return commands.to_vec();
    }

    let lower = query.to_lowercase();
    let mut scored: Vec<(i32, &CommandItem)> = commands
        .iter()
        .filter_map(|cmd| {
            let label = cmd.label.to_lowercase();
            let cat = cmd.category.to_lowercase();
            let combined = format!("{} {}", label, cat);

            // Score: exact substring match scores highest, then word starts
            if label.starts_with(&lower) {
                Some((100, cmd))
            } else if label.contains(&lower) {
                Some((80, cmd))
            } else if cat.starts_with(&lower) {
                Some((70, cmd))
            } else if combined.contains(&lower) {
                Some((60, cmd))
            } else {
                // Fuzzy: all query chars appear in order
                let mut chars = lower.chars();
                let mut current = chars.next();
                for c in combined.chars() {
                    if let Some(q) = current {
                        if c == q {
                            current = chars.next();
                        }
                    }
                }
                if current.is_none() {
                    Some((30, cmd))
                } else {
                    None
                }
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().take(15).map(|(_, cmd)| cmd.clone()).collect()
}

fn wire_search(ui: &App) {
    let commands = build_commands();
    let ui_weak = ui.as_weak();

    ui.on_command_palette_search(move |query| {
        if let Some(ui) = ui_weak.upgrade() {
            let filtered = filter_commands(&commands, query.as_str());
            ui.set_command_palette_filtered(ModelRc::new(VecModel::from(filtered)));
        }
    });
}

fn wire_selected(ui: &App, ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    let bridge = ctx.bridge.clone();

    ui.on_command_palette_selected(move |action_id| {
        let action = action_id.to_string();
        tracing::info!(action = %action, "Command palette: selected");

        let ui = match ui_weak.upgrade() {
            Some(u) => u,
            None => return,
        };

        // Close palette
        ui.set_command_palette_open(false);

        if action.starts_with("nav:") {
            if let Ok(screen) = action[4..].parse::<i32>() {
                ui.set_current_screen(screen);
                ui.invoke_navigate(screen);
            }
        } else if action == "system:lock" {
            ui.invoke_lock_screen();
        } else if action == "system:quick-settings" {
            ui.set_quick_settings_open(true);
        } else if action == "system:toggle-dnd" {
            ui.set_dnd_mode(!ui.get_dnd_mode());
        } else if action == "system:power" {
            ui.set_power_menu_open(true);
        } else if action == "system:notifications" {
            ui.set_current_screen(9);
            ui.invoke_navigate(9);
        } else if action == "system:about" {
            ui.set_current_screen(16);
            ui.invoke_navigate(16);
        } else if action.starts_with("email:") {
            ui.set_current_screen(17);
            ui.invoke_navigate(17);
        } else if action.starts_with("calendar:") {
            ui.set_current_screen(18);
            ui.invoke_navigate(18);
        } else if action.starts_with("notes:") {
            ui.set_current_screen(15);
            ui.invoke_navigate(15);
        } else if action.starts_with("terminal:") {
            ui.set_current_screen(14);
            ui.invoke_navigate(14);
        } else if action.starts_with("editor:") {
            ui.set_current_screen(12);
            ui.invoke_navigate(12);
        } else if action.starts_with("spreadsheet:") {
            ui.set_current_screen(29);
            ui.invoke_navigate(29);
        } else if action.starts_with("document:") {
            ui.set_current_screen(30);
            ui.invoke_navigate(30);
        } else if action.starts_with("presentation:") {
            ui.set_current_screen(31);
            ui.invoke_navigate(31);
        } else if action.starts_with("ai:") {
            // Open lens in chat mode for AI queries
            ui.set_lens_open(true);
            ui.invoke_open_lens();
        } else if action.starts_with("search:") {
            ui.set_lens_open(true);
            ui.invoke_open_lens();
        } else if action == "system:screenshot" {
            let _ = std::process::Command::new("grim")
                .arg("/tmp/screenshot.png")
                .spawn();
        } else if action == "system:toggle-theme" {
            let current = ui.global::<crate::ThemeMode>().get_dark();
            ui.global::<crate::ThemeMode>().set_dark(!current);
            ui.set_settings_dark_mode(!current);
        }
    });
}

fn wire_open(ui: &App) {
    let commands = build_commands();
    let ui_weak = ui.as_weak();

    ui.on_open_command_palette(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_command_palette_open(true);
            // Show all commands initially
            ui.set_command_palette_filtered(ModelRc::new(VecModel::from(commands.clone())));
        }
    });
}

// ── Helpers ──

fn nav(cmds: &mut Vec<CommandItem>, label: &str, screen: &str, icon: &str, action: &str) {
    cmds.push(CommandItem {
        label: SharedString::from(label),
        category: SharedString::from("Navigation"),
        shortcut: SharedString::default(),
        icon_char: SharedString::from(icon),
        action_id: SharedString::from(action),
    });
}

fn cmd(
    cmds: &mut Vec<CommandItem>,
    label: &str,
    category: &str,
    icon: &str,
    action: &str,
    shortcut: &str,
) {
    cmds.push(CommandItem {
        label: SharedString::from(label),
        category: SharedString::from(category),
        shortcut: SharedString::from(shortcut),
        icon_char: SharedString::from(icon),
        action_id: SharedString::from(action),
    });
}
