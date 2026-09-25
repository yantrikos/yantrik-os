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
    //
    // Only the screens `app.slint` draws. Sixteen rows stood here and nine of them went nowhere:
    // Terminal (14), Notes (15), Email (17), Calendar (18), Weather (19), Network (22), System
    // Monitor (23), Document Editor (30) and Presentation (31) all became their own app binaries,
    // their `if current-screen == N` branches went with them, and the palette rows stayed —
    // so choosing "Go to Terminal" set `current-screen` to a number nothing renders and left the
    // person looking at a blank screen with a taskbar on it, from which the only way back was
    // knowing to press the desktop key.
    //
    // They are gone rather than repointed: opening those apps is what the Lens and the launcher
    // are for, and a palette row is a promise about a place in the shell.
    // `every_navigation_command_names_a_screen_the_shell_has` below reads app.slint and fails if
    // one comes back.
    nav(&mut cmds, "Go to Desktop", "1", "⌂", "nav:1");
    nav(&mut cmds, "Go to Files", "8", "F", "nav:8");
    nav(&mut cmds, "Go to Settings", "7", "⚙", "nav:7");
    nav(&mut cmds, "Go to Packages", "21", "P", "nav:21");
    // Images, Text Editor and Media Player went the same way in #253, with the Editor's two
    // rows: the shell drew its own copy of each, and those copies are gone.

    // Music and ySheets are shelved, so the palette does not offer them. "Go to Music",
    // "Go to Spreadsheet", "New Spreadsheet" and "Import CSV" were here; see
    // wire::dock::SHELVED for why, and put them back with the apps.
    //
    // Email, Calendar, Notes, Terminal, Document and Presentation had rows here too — "Compose
    // New Email", "New Calendar Event", "New Note", "New Terminal Tab" (advertising Ctrl+T),
    // "New Document", "New Presentation" and three searches. Every one of them dispatched to a
    // screen id in the dead range above, so all ten did the same nothing the nav rows did. They
    // are not repointed at the apps either, because none of those binaries takes an instruction
    // on its command line: launching Notes and calling that "New Note" would be a second, quieter
    // version of the same broken promise.
    //
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

/// The screen a command lands on, for every command that is a navigation — whether or not it is
/// spelled `nav:`.
///
/// A function rather than arms inside the dispatch closure, because the closure needs a live
/// `App` and therefore cannot be called from a test, and the mapping is exactly what was wrong:
/// ten commands carried screen ids that `app.slint` stopped rendering when those apps became
/// their own binaries. `every_navigation_command_names_a_screen_the_shell_has` reads this and
/// app.slint together, so the next id to go stale fails the build instead of showing a blank
/// screen.
fn screen_for(action: &str) -> Option<i32> {
    if let Some(id) = action.strip_prefix("nav:") {
        return id.parse().ok();
    }
    match action {
        "system:notifications" => Some(9),
        "system:about" => Some(16),
        _ => None,
    }
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

        // The palette only draws on the desktop screen, but the rule is that nothing moves the
        // shell off login or lock on its own (#203), and this handler is one of the paths the
        // bug report names: every branch below navigates, locks, toggles or opens the Lens.
        // While the desktop waits for the person the choice is dropped, not queued — there is
        // no person's choice to keep, since the palette could not have been drawn.
        if crate::control::locked_screen(ui.get_current_screen()) {
            tracing::debug!(action = %action, "Command palette dropped — the desktop is waiting for the person to sign in");
            return;
        }

        // Close palette
        ui.set_command_palette_open(false);

        // Navigation first, from the one table a test can read. It used to be a chain of `else
        // if` arms holding screen ids inline — which is how ten of them came to point at screens
        // app.slint no longer draws without anything noticing.
        if let Some(screen) = screen_for(&action) {
            ui.set_current_screen(screen);
            ui.invoke_navigate(screen);
        } else if action == "system:lock" {
            ui.invoke_lock_screen();
        } else if action == "system:quick-settings" {
            ui.set_quick_settings_open(true);
        } else if action == "system:toggle-dnd" {
            ui.invoke_toggle_dnd_mode();
        } else if action == "system:power" {
            ui.set_power_menu_open(true);
        } else if action.starts_with("ai:") || action.starts_with("search:") {
            // The Lens, which the desktop screen draws and no other screen does. Six rows set
            // `lens-open` and stopped there, so choosing "Ask AI" or "Search Files" from any
            // screen but the desktop flipped a property behind a screen that does not render the
            // panel: the palette closed and nothing else happened. Same two lines the ask bar,
            // the orb and Ctrl+K use.
            if ui.get_current_screen() != 1 {
                ui.set_current_screen(1);
                ui.invoke_navigate(1);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The palette does not offer a shelved app, by any of its names.
    ///
    /// The palette is a hardcoded list rather than a view of the catalogue, so nothing filters it
    /// on the way to the screen: a row here is shown whatever the shelf says. This is the check
    /// that stands in for that filter.
    #[test]
    fn no_command_offers_a_shelved_app() {
        for item in build_commands() {
            let label = item.label.to_string();
            let action = item.action_id.to_string();
            for word in label.split_whitespace().chain(action.split(':')) {
                assert!(
                    crate::wire::dock::shelved(word).is_none(),
                    "the palette offers `{label}` ({action}), and `{word}` is shelved"
                );
            }
        }
    }

    /// The screen ids `app.slint` actually draws, read off the file.
    ///
    /// The same source of truth `control::screen_table_tests` reads, for the same reason: a
    /// number means whatever that file says it means, and any list of ids kept by hand beside it
    /// goes stale without anyone touching it.
    fn rendered_screens() -> Vec<i32> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yantrik-ui-slint/ui/app.slint");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        src.lines()
            .filter_map(|line| line.trim().strip_prefix("if current-screen == "))
            .filter_map(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i32>().ok()
            })
            .collect()
    }

    /// Every command goes somewhere. A palette row that navigates to a screen nothing renders is
    /// the same broken promise as a tile that opens nothing.
    ///
    /// This used to assert that no row named screen 20 or 29 and called the rest "older debt".
    /// The older debt was nine nav rows and ten app commands pointing at 14, 15, 17, 18, 19, 22,
    /// 23, 30 and 31 — every one of which had stopped being drawn when those apps became their
    /// own binaries. The check now reads app.slint instead of naming the two ids somebody
    /// happened to have looked at.
    #[test]
    fn every_navigation_command_names_a_screen_the_shell_has() {
        let rendered = rendered_screens();
        assert!(!rendered.is_empty(), "app.slint has no screen branches; the reader is broken");

        let dead: Vec<String> = build_commands()
            .iter()
            .filter_map(|item| {
                let screen = screen_for(item.action_id.as_str())?;
                (!rendered.contains(&screen))
                    .then(|| format!("{} -> screen {screen}", item.label))
            })
            .collect();

        assert!(
            dead.is_empty(),
            "these palette rows navigate to screens app.slint does not draw, so choosing one \
             leaves a blank screen:\n  {}\n\n\
             Either the screen went away (drop the row) or the id is wrong. The ids are defined \
             by app.slint, not by this file.",
            dead.join("\n  ")
        );
    }

    /// A `nav:` row carries a number, not a name. Parsing it in the dispatch and silently doing
    /// nothing when it fails is how a typo becomes a row that looks fine and is not.
    #[test]
    fn every_nav_row_carries_a_screen_id_that_parses() {
        for item in build_commands() {
            let action = item.action_id.to_string();
            if action.starts_with("nav:") {
                assert!(
                    screen_for(&action).is_some(),
                    "`{}` is a nav row whose action `{action}` has no screen id",
                    item.label
                );
            }
        }
    }
}
