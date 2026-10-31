//! The Bench's callbacks: what the pointer and the keys said, handed to the model and the
//! operations, and the result put back on the screen.
//!
//! Every handler goes through [`run`], which holds the Bench for the length of the call and
//! publishes after it. A handler never calls Slint while it holds data it would need again.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;
use yantrik_bench::layout::METRICS;

use super::menu::{self, Context, Pick};
use super::{ops, view, Bench, Root};
use crate::{App, BenchState};

type Shared = Rc<RefCell<Bench>>;

/// Run `f` on the Bench with the window, then redraw what changed. A call that arrives while the
/// Bench is already in use (a handler that opens an app, which reports back) is dropped, not
/// queued: it is the echo of the first.
fn run(shared: &Shared, weak: &slint::Weak<App>, f: impl FnOnce(&App, &mut Bench)) {
    let Some(ui) = weak.upgrade() else { return };
    let Ok(mut b) = shared.try_borrow_mut() else {
        tracing::debug!("The Bench was busy; a callback was dropped");
        return;
    };
    f(&ui, &mut b);
    view::publish(&ui, &mut b);
}

pub fn close_menu(ui: &App) {
    let state = ui.global::<BenchState>();
    state.set_menu_open(false);
    state.set_menu_keep(false);
}

fn show_menu(ui: &App, rows: &[menu::Row], x: f32, y: f32, keep: bool) {
    let state = ui.global::<BenchState>();
    state.set_menu_rows(slint::ModelRc::new(slint::VecModel::from(view::menu_rows(rows))));
    if !keep {
        state.set_menu_x(x);
        state.set_menu_y(y);
    }
    state.set_menu_keep(keep);
    state.set_menu_open(true);
}

fn context(b: &mut Bench) -> Context {
    let items = b.model.selected_items();
    let (only_folder, only_app) = (
        !items.is_empty() && items.iter().all(|i| i.kind == yantrik_bench::items::Kind::Folder),
        !items.is_empty() && items.iter().all(|i| i.kind == yantrik_bench::items::Kind::App),
    );
    let selected = items.len();
    let (pinnable, pinned) = if selected == 1 && only_app { ops::pin_state(b) } else { (false, false) };
    Context {
        selected,
        only_folder,
        only_app,
        pinnable,
        pinned,
        can_paste: ops::can_paste(),
        snap: b.model.store.snap,
        icons_shown: b.model.show_icons(),
    }
}

pub fn wire(ui: &App, shared: Shared) {
    let state = ui.global::<BenchState>();
    let weak = ui.as_weak();
    // The numbers the layout arithmetic uses, so a drop lands where it is drawn.
    state.set_cell_w(METRICS.cell_w as f32);
    state.set_cell_h(METRICS.cell_h as f32);
    state.set_pad_x(METRICS.pad_x as f32);
    state.set_pad_y(METRICS.pad_y as f32);
    state.set_tile(56.0);

    macro_rules! on {
        ($setter:ident, |$ui:ident, $b:ident $(, $arg:ident)*| $body:block) => {{
            let (shared, weak) = (shared.clone(), weak.clone());
            state.$setter(move |$($arg),*| run(&shared, &weak, |$ui, $b| $body));
        }};
    }

    {
        let (shared, weak) = (shared.clone(), weak.clone());
        state.on_area_changed(move |w, h| {
            // `set_area` redraws only when the icons moved; `run` publishes in any case, which
            // is one comparison of the model when nothing changed.
            run(&shared, &weak, |_ui, b| {
                b.model.set_area(w as i32, h as i32);
            });
        });
    }
    on!(on_select, |_ui, b, id, ctrl| {
        close_menu(_ui);
        b.root = None;
        b.model.renaming = None;
        b.model.press(id.as_str(), ctrl);
    });
    on!(on_click_released, |_ui, b, id, ctrl| {
        b.model.click_release(id.as_str(), ctrl);
    });
    on!(on_open_item, |ui, b, id| {
        ops::open_one(ui, b, id.as_str());
    });
    on!(on_move_items, |_ui, b, _id, dx, dy| {
        b.model.move_selected(dx.round() as i32, dy.round() as i32);
    });
    on!(on_rubber_band, |_ui, b, x, y, w, h, ctrl| {
        b.model.select_rect(x as i32, y as i32, w as i32, h as i32, ctrl);
    });
    on!(on_background_pressed, |ui, b, ctrl| {
        close_menu(ui);
        b.root = None;
        b.model.renaming = None;
        if !ctrl {
            b.model.clear_selection();
        }
    });
    on!(on_item_menu, |ui, b, id, x, y| {
        // A right-click on an icon that is not selected selects it alone, as on every desktop.
        if !b.model.selection.contains(id.as_str()) {
            b.model.selection.clear();
            b.model.press(id.as_str(), false);
        }
        b.root = Some(Root::Item);
        let rows = menu::item(&context(b));
        show_menu(ui, &rows, x, y, false);
    });
    on!(on_background_menu, |ui, b, x, y| {
        b.model.clear_selection();
        b.model.renaming = None;
        b.root = Some(Root::Desktop);
        let rows = menu::desktop(&context(b));
        show_menu(ui, &rows, x, y, false);
    });
    on!(on_menu_action, |ui, b, id| {
        act(ui, b, id.as_str());
    });
    on!(on_rename_commit, |ui, b, id, text| {
        ops::commit_rename(ui, b, id.as_str(), text.as_str());
    });
    on!(on_rename_cancel, |_ui, b| {
        b.model.renaming = None;
    });
    on!(on_continue_open, |ui, b| {
        ops::open_continue(ui, b);
    });
    // `key` answers, so it cannot go through `on!`.
    {
        let (shared, weak) = (shared.clone(), weak.clone());
        state.on_key(move |name, ctrl, shift| {
            let Some(ui) = weak.upgrade() else { return false };
            let Ok(mut b) = shared.try_borrow_mut() else { return false };
            super::keys::handle(&ui, &mut b, name.as_str(), ctrl, shift)
        });
    }
}

/// A menu row was chosen.
fn act(ui: &App, b: &mut Bench, id: &str) {
    let Some(pick) = menu::pick(id) else { return };
    if menu::is_submenu(&pick) {
        // The menu stays up with other rows; the click that chose this row must not close it.
        let rows = match pick {
            Pick::ArrangeMenu => menu::arrange(b.model.store.snap),
            Pick::OpenWithMenu => menu::open_with(&ops::open_with_apps(b)),
            _ => match b.root {
                Some(Root::Item) => menu::item(&context(b)),
                _ => menu::desktop(&context(b)),
            },
        };
        show_menu(ui, &rows, 0.0, 0.0, true);
        return;
    }
    close_menu(ui);
    b.root = None;
    match pick {
        Pick::NewFolder => ops::new_thing(ui, b, true),
        Pick::NewFile => ops::new_thing(ui, b, false),
        Pick::Paste => ops::paste(ui, b),
        Pick::Snap(on) => b.model.set_snap(on),
        Pick::Sort(how) => b.model.sort(how),
        Pick::ChangeWallpaper => ops::change_wallpaper(ui),
        Pick::ToggleIcons => {
            let on = !b.model.show_icons();
            b.model.set_show_icons(on);
        }
        Pick::Launch(app) => ui.invoke_launch_app(app.into()),
        Pick::Open => ops::open_selection(ui, b),
        Pick::OpenWith(app) => ops::open_with(ui, b, &app),
        Pick::Rename => ops::start_rename(ui, b),
        Pick::Reveal => ops::reveal(ui, b),
        Pick::Archive => ops::archive_selection(ui, b),
        Pick::TogglePin => ops::toggle_pin(ui, b),
        Pick::ArrangeMenu | Pick::OpenWithMenu | Pick::Back => {}
    }
}
