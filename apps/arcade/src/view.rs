//! The window: its callbacks bound to commands, and the model drawn for the parts that changed.
//! The only code that writes the window.

use slint::SharedString;
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::rows;

use crate::command::Command;
use crate::model::{detail_line, Model, PART_EDITOR, PART_LISTS, PART_NOTICE};
use crate::{Arcade, ArcadeApp, CharacterRow, GameRow};

pub fn wire(cx: &Cx<Arcade>) {
    let ui = cx.ui();
    // One callback for every button, from the header or the workbench: the window and the control
    // surface both arrive at `update` and nowhere else.
    ui.on_action(cx.cmd1(|id: SharedString| Command::Button(id.to_string())));
    ui.on_pick_game(cx.cmd1(|slug: SharedString| Command::PickGame(slug.to_string())));
    ui.on_pick_character(cx.cmd1(|slug: SharedString| Command::PickCharacter(slug.to_string())));
    ui.on_spec_edited(cx.cmd1(|text: SharedString| Command::SpecEdited(text.to_string())));
    ui.on_game_edited(cx.cmd1(|text: SharedString| Command::GameEdited(text.to_string())));
}

pub fn render(ui: &ArcadeApp, m: &Model, changed: &Changed) {
    if changed.has(PART_LISTS) {
        lists(ui, m);
    }
    if changed.has(PART_NOTICE) && ui.get_notice().as_str() != m.notice {
        ui.set_notice(m.notice.as_str().into());
    }
    // The fields the person types in: set only where they differ, so the caret stays where it is.
    if changed.has(PART_EDITOR) {
        if ui.get_spec_text().as_str() != m.spec_text {
            ui.set_spec_text(m.spec_text.as_str().into());
        }
        if ui.get_game_name().as_str() != m.game_name {
            ui.set_game_name(m.game_name.as_str().into());
        }
    }
}

/// Both lists, the status line and whether the buttons are held.
fn lists(ui: &ArcadeApp, m: &Model) {
    let selected = &m.game_name;
    let games: Vec<GameRow> = m
        .listing
        .games
        .iter()
        .map(|g| GameRow {
            slug: g.slug.clone().into(),
            title: g.title.clone().into(),
            detail: detail_line(g).into(),
            selected: &g.slug == selected || &g.title == selected,
        })
        .collect();
    // Set only when a row differs: a list at rest is not drawn again.
    rows::set(ui.get_games(), games, |model| ui.set_games(model));
    let characters: Vec<CharacterRow> = m
        .listing
        .characters
        .iter()
        .map(|c| CharacterRow {
            slug: c.slug.clone().into(),
            name: c.name.clone().into(),
            archetype: c.archetype.clone().into(),
            selected: false,
        })
        .collect();
    rows::set(ui.get_characters(), characters, |model| ui.set_characters(model));

    let status = m.status();
    if ui.get_status().as_str() != status {
        ui.set_status(status.into());
    }
    let busy = !m.jobs.running().is_empty();
    if ui.get_busy() != busy {
        ui.set_busy(busy);
    }
}
