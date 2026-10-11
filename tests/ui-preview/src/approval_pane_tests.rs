//! The largest approval card, hosted in an agent's own pane (agents.slint, `ItemView`), at the
//! window size `verify-approval-fit` is given. The pane scrolls itself, and until the follow-up to
//! #639 its card had no height limit: the read gate never applied there, and the pane's scroll
//! could put the arguments out of view while Allow was on screen. Now the pane passes its
//! scroller's visible height as the card's limit, so the card takes the same three cases as in
//! the Lens and the corner.
//!
//! The pane holds the agent's prompt above the card, so its content is taller than the scroller
//! and follows the bottom. Decline answers inside the pane, above its reply box; the whole card,
//! its top edge included, is in view at once; and where the card had to scroll, Allow and the
//! session row grant nothing until the card's own scroll has been to its end.
use super::agents_tests::{fill, model};
use super::approval_fit_tests::{answer, largest};
use super::approval_tests::{save, scan, settle};
use super::*;
use std::cell::Cell;

pub fn run(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (fw, fh) = (width as f32, height as f32);
    let window = AgentWindow::new()?;
    window.set_agent_title("Agent · pi · open a terminal".into());
    let g = window.global::<AgentsState>();
    fill(&g, true);
    let (allowed, denied, sessioned) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    {
        let (a, d, s) = (allowed.clone(), denied.clone(), sessioned.clone());
        g.on_approval_allow(move |_| a.set(a.get() + 1));
        g.on_approval_deny(move |_| d.set(d.get() + 1));
        g.on_approval_allow_session(move |_| s.set(s.get() + 1));
    }
    let item = |kind: &str, key: &str, text: &str| AgentItemData {
        kind: kind.into(),
        key: key.into(),
        text: text.into(),
        ..Default::default()
    };
    g.set_items(model(vec![
        item("prompt", "t3", "open a terminal for me"),
        item("text", "t3.0", "Asking before it runs."),
        AgentItemData { approval: largest(), ..item("approval", "t3.1", "shell.run_recipe") },
    ]));
    window.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    std::thread::sleep(std::time::Duration::from_millis(300));
    let first = settle(w, width, height);
    save(&first, &output.replace(".png", "-pane.png"), width, height)?;

    // The window's 12px padding, the session's 12px, its 26px header, a separator: the scroller
    // starts about 70px down. Its foot is the reply box's top, about 60px above the window's.
    // The session is the window less the 250px details column and the gaps; the card spans it,
    // less the list's 6px on the right, Decline on the left half and Allow on the right.
    let (pane_top, pane_bottom) = (60.0f32, fh - 60.0);
    let (left, right) = (36.0f32, fw - 304.0);
    let quarter = (right - left) / 4.0;
    let (deny_x, allow_x, mid_x) = (left + quarter, right - quarter, (left + right) / 2.0);

    // Where Decline is, to know where the card's top part is: the card is no taller than the
    // scroller, so it is all in view, and its top part lies above the buttons.
    let before = denied.get();
    let deny_y = scan(w, deny_x, pane_top, pane_bottom, || denied.get() > before)
        .unwrap_or_else(|| panic!("pane at {width}×{height}: Decline answers inside the pane"));
    let in_pane = answer(
        w,
        &format!("pane at {width}×{height}"),
        (deny_x, allow_x, mid_x, (pane_top + deny_y) / 2.0),
        (pane_top, pane_bottom),
        (width, height),
        &|| denied.get(),
        &|| allowed.get(),
        &|| sessioned.get(),
    );
    assert!(in_pane.deny_y < pane_bottom && in_pane.allow_y < pane_bottom, "both above the reply box, inside the pane");
    if (width, height) == (800, 600) {
        assert!(in_pane.waited, "at 800×600 the largest card has to scroll in the pane, and Allow waits for its end");
    }

    // The whole card is in view at once: its amber edge (a sensitive card's) is drawn as a line
    // across the card both above the buttons and below them, inside the pane. A card taller than
    // the scroller would have one of the two beyond the pane's edge.
    let shown = settle(w, width, height);
    save(&shown, &output.replace(".png", "-pane-read.png"), width, height)?;
    let edge_row = |y: u32| {
        let xs = (left as u32 + 24)..(right as u32 - 24);
        let n = xs.len();
        let amber = xs
            .filter(|x| {
                let p = shown.as_slice()[(y * width + x) as usize];
                p.r >= 150 && p.r as i32 > p.b as i32 + 60
            })
            .count();
        amber * 10 >= n * 9
    };
    let top_edge = (pane_top as u32..in_pane.deny_y as u32).find(|y| edge_row(*y));
    let foot_edge = (in_pane.deny_y as u32..pane_bottom as u32).find(|y| edge_row(*y));
    assert!(
        top_edge.is_some() && foot_edge.is_some(),
        "pane at {width}×{height}: the card's top ({top_edge:?}) and foot ({foot_edge:?}) are both inside the pane"
    );
    window.hide()?;
    println!(
        "pane at {width}×{height}: the largest card keeps Decline and Allow inside the pane (Decline at {}){}",
        in_pane.deny_y,
        if in_pane.waited { ", Allow after reading to the end" } else { "" },
    );
    Ok(())
}
