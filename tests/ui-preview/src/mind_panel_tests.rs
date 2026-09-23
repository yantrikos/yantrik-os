//! The mind panel, drawn by the whole shell — app.slint's `App`, not a copy of its layout — from
//! fixture data, with real pointer events: an agent's row opens that agent in the Agents screen,
//! the strip is there over another screen and opens the panel, the chevron folds it, and it is on
//! the Files screen too — where, open, the maximized Files window stops short of it. Renders the
//! desktop (dark, light, agent mode) and the other screens with the strip and with the panel open
//! beside them.
use super::*;
use slint::{ModelRc, VecModel};
use std::cell::RefCell;

pub(crate) fn fill(ui: &App) {
    ui.set_clock_text("10:24".into());
    ui.set_date_text("Wednesday, September 23".into());
    ui.set_greeting_text("Good morning".into());
    ui.set_wallpaper_path("serenity".into());
    ui.set_companion_online(true);
    ui.set_network_online(true);
    ui.set_active_harness_id("companion".into());
    ui.set_active_harness_name("Yantrik Companion".into());
    ui.set_mind_mode("ask".into());
    ui.set_mind_mode_label("Ask".into());
    ui.set_mind_ceiling("sensitive".into());
    ui.set_bar_cpu_percent(12);
    ui.set_bar_mem_percent(41);
    ui.set_bar_mem_text("3.1 GB".into());
    ui.set_bar_swap_percent(0);
    ui.set_bar_swap_text("0 B".into());
    ui.set_bar_disk_percent(58);
    ui.set_bar_disk_text("41 GB".into());
    ui.set_ambient_interval_ms(0);
    let pin = |id: &str, label: &str| DockItem { app_id: id.into(), icon_id: id.into(), label: label.into(), ..Default::default() };
    ui.set_dock_items(ModelRc::new(VecModel::from(vec![
        pin("files", "Files"),
        pin("notes", "Notes"),
        pin("terminal", "Terminal"),
        pin("calendar", "Calendar"),
        pin("settings", "Settings"),
    ])));
    let service = |id: &str, status: &str, note: &str| ServiceItem { id: id.into(), status: status.into(), note: note.into() };
    ui.set_services(ModelRc::new(VecModel::from(vec![
        service("network", "running", "up"),
        service("notifications", "running", "up"),
        service("notes", "stopped", "on demand"),
    ])));

    let g = ui.global::<MindPanelState>();
    g.set_now(MindPanelNow {
        known: true,
        mind: "Yantrik Companion".into(),
        initial: "Y".into(),
        builtin: true,
        model: "qwen3.5:9b".into(),
        model_named: true,
        state: "ready".into(),
        project: "".into(),
        memory: "YantrikDB · 1,234 memories".into(),
        memory_known: true,
        minds: "3 minds · 2 keep memory".into(),
        running: 2,
        needs_you: 1,
        more_agents: 0,
        recipes_known: true,
        services: "2 running · 1 stopped".into(),
        services_trouble: false,
    });
    let agent = |id: &str, mind: &str, title: &str, state: &str, label: &str, since: &str| MindPanelAgent {
        id: id.into(),
        mind: mind.into(),
        title: title.into(),
        state: state.into(),
        label: label.into(),
        since: since.into(),
    };
    g.set_agents(ModelRc::new(VecModel::from(vec![
        agent("deepseek:main", "DeepSeek", "release notes for 0.4", "waiting_for_you", "waiting for you", "40s"),
        agent("pi:main", "pi", "tidy the photos folder, dupes into Trash", "running_tool", "running a tool", "2m"),
    ])));
    g.set_recipes(ModelRc::new(VecModel::from(vec![MindPanelRecipe {
        id: "rcp_1".into(),
        name: "Morning digest".into(),
        step: "step 2 of 3 · running web_search".into(),
        status: "running".into(),
    }])));
    let act = |what: &str, when: &str, outcome: &str| MindPanelAct { what: what.into(), when: when.into(), outcome: outcome.into() };
    g.set_acts(ModelRc::new(VecModel::from(vec![
        act("files.move", "4m ago", "ok"),
        act("notes.create", "12m ago", "ok"),
        act("calendar.add_event", "1h ago", "failed"),
    ])));
}

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({width}×{height})");
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (1280u32, 800u32);
    let ui = App::new()?;
    ui.set_current_screen(1);
    fill(&ui);

    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let a = ui.global::<AgentsState>();
        let l = log.clone();
        a.on_select(move |id| l.borrow_mut().push(format!("select:{id}")));
        let l = log.clone();
        a.on_select_tab(move |id| l.borrow_mut().push(format!("tab:{id}")));
        // The shell's own handler, minus the file: record it and show it.
        let weak = ui.as_weak();
        let l = log.clone();
        ui.global::<MindPanelState>().on_set_expanded(move |place, open| {
            l.borrow_mut().push(format!("expand:{place}:{open}"));
            if let Some(ui) = weak.upgrade() {
                let g = ui.global::<MindPanelState>();
                if place == "desktop" { g.set_desktop_expanded(open) } else { g.set_elsewhere_expanded(open) }
            }
        });
    }
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), width as usize); });
        pixels
    };
    let settle = || {
        // Past the shell windows' 200 ms geometry animation.
        std::thread::sleep(std::time::Duration::from_millis(260));
        draw()
    };
    let path = |suffix: &str| output.replace(".png", &format!("-{suffix}.png"));

    // The desktop: the panel open by default.
    draw();
    save(&settle(), output, width, height)?;
    assert!(ui.global::<MindPanelState>().get_desktop_expanded(), "open on the desktop by default");

    // #184: the mode menu, as `show_mind_audit` leaves it, belongs to the screen it was opened
    // over — a screen change, whatever made it, puts it away with its sub-panels.
    ui.set_mind_menu_confirming(false);
    ui.set_mind_menu_audit_open(true);
    ui.set_mind_menu_open(true);
    draw();
    save(&settle(), &path("mode-menu"), width, height)?;
    ui.set_current_screen(8);
    draw();
    assert!(!ui.get_mind_menu_open() && !ui.get_mind_menu_audit_open(), "a screen change puts the mode menu away");
    ui.set_current_screen(1);
    draw();

    // An agent's row opens that agent in the Agents screen. Swept down the Working section's rows
    // only, and stopped the moment the shell leaves the desktop.
    let x = 1110.;
    let mut y = 296.;
    while y < 420. && ui.get_current_screen() == 1 {
        click(w, x, y);
        draw();
        y += 4.;
    }
    assert_eq!(ui.get_current_screen(), 34, "a row went to the Agents screen; log {:?}", log.borrow());
    assert!(log.borrow().contains(&"tab:active".to_string()), "on the Active tab: {:?}", log.borrow());
    assert!(log.borrow().contains(&"select:deepseek:main".to_string()), "with the agent that needs the person selected: {:?}", log.borrow());

    // A recipe's row opens the Recipes screen with that recipe opened. Swept down past the agent
    // rows, which open Agents; the sweep goes back to the desktop after each of those.
    {
        let l = log.clone();
        ui.global::<RecipesState>().on_show(move |id| l.borrow_mut().push(format!("show-recipe:{id}")));
        ui.global::<RecipesState>().set_rows(ModelRc::new(VecModel::from(super::recipes_tests::rows())));
        ui.global::<RecipesState>().set_loaded(true);
    }
    let mut y = 296.;
    ui.set_current_screen(1);
    draw();
    while y < 600. && ui.get_current_screen() != 35 {
        click(w, x, y);
        draw();
        if ui.get_current_screen() != 1 && ui.get_current_screen() != 35 {
            ui.set_current_screen(1);
            draw();
        }
        y += 4.;
    }
    assert_eq!(ui.get_current_screen(), 35, "a recipe row went to the Recipes screen; log {:?}", log.borrow());
    assert!(log.borrow().contains(&"show-recipe:rcp_1".to_string()), "with that recipe shown: {:?}", log.borrow());
    // The Recipes screen in the shell: maximized, it stops short of the panel's strip.
    save(&settle(), &path("recipes-strip"), width, height)?;
    ui.set_current_screen(34);
    draw();

    // Over another screen: the strip, which opens the panel for everywhere-but-the-desktop.
    save(&settle(), &path("agents-strip"), width, height)?;
    click(w, 1258., 460.);
    draw();
    assert!(log.borrow().contains(&"expand:elsewhere:true".to_string()), "the strip is on the Agents screen and opens it: {:?}", log.borrow());
    save(&settle(), &path("agents-open"), width, height)?;

    // The chevron folds it again.
    let before = log.borrow().len();
    for yy in [74., 80., 84., 88., 94.] {
        click(w, 1236., yy);
        draw();
        if log.borrow().len() > before {
            break;
        }
    }
    assert!(log.borrow()[before..].contains(&"expand:elsewhere:false".to_string()), "the chevron folds it: {:?}", log.borrow());

    // And it is on Files: furniture, not a desktop widget.
    ui.set_current_screen(8);
    draw();
    save(&settle(), &path("files-strip"), width, height)?;
    let before = log.borrow().len();
    click(w, 1258., 460.);
    draw();
    assert!(log.borrow()[before..].contains(&"expand:elsewhere:true".to_string()), "the strip is on Files too: {:?}", log.borrow());
    // Open beside Files: the maximized Files window stops short of the card instead of running
    // under it. Its × is the last thing in its title bar, so it must now sit left of the card.
    save(&settle(), &path("files-open"), width, height)?;
    for x in [1244., 1250., 1256.] {
        click(w, x, 50.);
        draw();
    }
    assert_eq!(ui.get_current_screen(), 8, "the far right of the title-bar row is the panel's room, not Files' ×");
    ui.global::<MindPanelState>().set_elsewhere_expanded(false);

    // The desktop in the light theme, and in agent mode (where the machine's load and services live).
    ui.set_current_screen(1);
    ui.global::<ThemeMode>().set_dark(false);
    draw();
    save(&settle(), &path("light"), width, height)?;
    ui.global::<ThemeMode>().set_dark(true);
    ui.set_agent_mode(true);
    draw();
    save(&settle(), &path("agent-mode"), width, height)?;

    // Nothing at work and nothing known yet: the panel says so rather than drawing zeros.
    ui.set_agent_mode(false);
    let g = ui.global::<MindPanelState>();
    g.set_now(MindPanelNow {
        known: false,
        mind: "unknown".into(),
        initial: "?".into(),
        model: "unknown".into(),
        state: "unknown".into(),
        memory: "YantrikDB · count not known yet".into(),
        minds: "not known yet".into(),
        services: "none registered".into(),
        ..Default::default()
    });
    g.set_agents(ModelRc::new(VecModel::from(Vec::<MindPanelAgent>::new())));
    g.set_recipes(ModelRc::new(VecModel::from(Vec::<MindPanelRecipe>::new())));
    g.set_acts(ModelRc::new(VecModel::from(Vec::<MindPanelAct>::new())));
    draw();
    save(&settle(), &path("unknown"), width, height)?;
    ui.hide()?;
    println!("PASS: panel open on the desktop; an agent row opens it in Agents (Active tab, selected); a recipe row opens it in Recipes; the strip is on Agents and Files and opens the panel; the chevron folds it; light, agent-mode and unknown states rendered");
    Ok(())
}
