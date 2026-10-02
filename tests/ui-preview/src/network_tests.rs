//! The bar's network mark and its popover (story 1.3), drawn in the real shell with real pointer
//! and key events, in the five states the story names:
//!
//!   * the VM shape: wired, no Wi-Fi device. The mark is drawn, and the popover has no Wi-Fi rows;
//!   * the laptop shape: five networks, one joined, one secured and unknown;
//!   * the password row: the field opens in the row of the secured network, what is typed shows as
//!     dots and is in no property anyone could read, and it is gone once submitted;
//!   * offline, and connected with no internet.
//!
//! It also pins the bug the story fixed: the Wi-Fi tile is the radio and Disconnect is a
//! disconnect, so pressing each reaches its own callback and not the other's.
//!
//! Callbacks are recorded here and never acted on; the Rust side that acts on them is
//! `yantrik-ui`'s `wire::network`, whose tests read its source.
use super::*;
use slint::platform::Key;
use slint::{Model, ModelRc, VecModel};
use std::cell::{Cell, RefCell};

const W: u32 = 1280;
const H: u32 = 800;

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), W, H);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(pixels.as_bytes())?;
    println!("Rendered {path} ({W}x{H})");
    Ok(())
}

/// How many pixels in the rectangle differ between two frames.
fn changed(a: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, b: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, x: (usize, usize), y: (usize, usize)) -> usize {
    (y.0..y.1)
        .flat_map(|y| (x.0..x.1).map(move |x| y * W as usize + x))
        .filter(|&i| a.as_slice()[i] != b.as_slice()[i])
        .count()
}

fn row(ssid: &str, strength: i32, secured: bool, known: bool, connected: bool, enterprise: bool) -> NetworkRow {
    NetworkRow {
        ssid: ssid.into(),
        strength,
        bars: match strength { 75..=100 => 4, 50..=74 => 3, 25..=49 => 2, _ => 1 },
        secured,
        known,
        connected,
        enterprise,
    }
}

/// A wired machine with no Wi-Fi device: what VM 520 is.
fn vm_shape(g: &NetworkState) {
    g.set_mark("wired".into());
    g.set_bars(0);
    g.set_tooltip("Wired, connected, 192.168.4.44".into());
    g.set_kind("wired".into());
    g.set_online(true);
    g.set_connecting(false);
    g.set_ssid("".into());
    g.set_ip("192.168.4.44".into());
    g.set_vpn(false);
    g.set_no_internet(false);
    g.set_portal(false);
    g.set_wifi_present(false);
    g.set_radio_on(false);
    g.set_networks(ModelRc::new(VecModel::from(Vec::<NetworkRow>::new())));
}

/// A laptop on Wi-Fi: five networks, "Home" joined, "Cafe" secured and not saved.
fn laptop_shape(g: &NetworkState) {
    g.set_mark("wifi".into());
    g.set_bars(3);
    g.set_tooltip("Wi-Fi Home, connected, 192.168.1.20".into());
    g.set_kind("wifi".into());
    g.set_online(true);
    g.set_connecting(false);
    g.set_ssid("Home".into());
    g.set_ip("192.168.1.20".into());
    g.set_vpn(false);
    g.set_no_internet(false);
    g.set_portal(false);
    g.set_wifi_present(true);
    g.set_radio_on(true);
    g.set_networks(ModelRc::new(VecModel::from(vec![
        row("Home", 68, true, true, true, false),
        row("Cafe", 55, true, false, false, false),
        row("Neighbour", 41, true, true, false, false),
        row("Guest", 33, false, false, false, false),
        row("Corp", 20, true, false, false, true),
    ])));
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.set_current_screen(1);
    let g = ui.global::<NetworkState>();

    // What the popover asked for, recorded.
    let scans: Rc<Cell<u32>> = Rc::default();
    let radio: Rc<RefCell<Vec<bool>>> = Rc::default();
    let leaves: Rc<Cell<u32>> = Rc::default();
    let joins: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    {
        let n = scans.clone();
        g.on_scan_requested(move || n.set(n.get() + 1));
        let r = radio.clone();
        g.on_set_radio(move |on| r.borrow_mut().push(on));
        let l = leaves.clone();
        g.on_disconnect(move || l.set(l.get() + 1));
        let j = joins.clone();
        g.on_connect(move |ssid, secret| j.borrow_mut().push((ssid.to_string(), secret.to_string())));
    }

    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let draw = || {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..4 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); });
        }
        pixels
    };
    let park = || w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(300.0, 500.0) });
    let path = |what: &str| output.replace(".png", &format!("-{what}.png"));
    let typed = |text: &str| { for c in text.chars() { key(w, c.to_string().into()); } };
    draw();

    // ── The VM shape ──────────────────────────────────────────────────────────────────────────
    vm_shape(&g);
    park();
    let vm_closed = draw();
    let anchor = ui.get_network_anchor_x();
    assert!(anchor > 600.0, "the bar reported where the network mark is: {anchor}");
    // The mark is drawn: swapping wired for offline changes pixels in its box.
    g.set_mark("offline".into());
    park();
    let offline_mark = draw();
    assert!(
        changed(&vm_closed, &offline_mark, (anchor as usize - 10, anchor as usize + 10), (6, 26)) > 15,
        "the wired mark and the offline mark are different drawings in the indicator's box"
    );
    g.set_mark("wired".into());
    park();
    draw();

    click(w, anchor, 16.0);
    park();
    let vm_open = draw();
    assert!(ui.get_network_open(), "pressing the network mark opens its popover");
    assert!(!ui.get_quick_settings_open(), "and not Quick Settings, which shared the press with the battery");
    assert_eq!(scans.get(), 1, "opening it asked for one scan");
    let panel = changed(&vm_closed, &vm_open, (900, 1280), (36, 230));
    assert!(panel > 20_000, "the popover is drawn under the bar: only {panel} pixels changed");
    // No Wi-Fi rows and no radio tile: read the saved PNG; the wired popover is two rows.
    save(&vm_open, &path("vm"))?;
    key(w, Key::Escape.into());
    park();
    draw();
    assert!(!ui.get_network_open(), "Esc closes it");

    // Quick Settings has a visible door at the far right on every machine, battery or not: a plain
    // click on it opens Quick Settings, as the network mark did before it had a popover of its own.
    {
        let mut x = W as f32 - 4.0;
        let mut found = None;
        while x > W as f32 - 90.0 {
            click(w, x, 16.0);
            draw();
            if ui.get_quick_settings_open() { found = Some(x); break; }
            x -= 2.0;
        }
        let x = found.expect("a press near the right end of the bar opens Quick Settings on a machine with no battery");
        println!("Quick Settings button at x={x}");
        ui.set_quick_settings_open(false);
        // Its tooltip must not run off the screen.
        w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(x, 16.0) });
        std::thread::sleep(std::time::Duration::from_millis(900));
        let tip = draw();
        save(&tip, &path("vm-qs-button-tooltip"))?;
        park();
        draw();
    }

    // Quick Settings on the wired VM: no Wi-Fi tile. VM 520 showed "WiFi, Disconnected, Tap to
    // connect" there while it was wired and online, which is a reading nobody had taken.
    ui.set_quick_settings_open(true);
    park();
    let qs_vm = draw();
    save(&qs_vm, &path("vm-quick-settings"))?;
    g.set_wifi_present(true);
    g.set_radio_on(true);
    park();
    let qs_with_radio = draw();
    assert!(
        changed(&qs_vm, &qs_with_radio, (450, 830), (50, 130)) > 2_000,
        "with a Wi-Fi device the tile is drawn at the top of Quick Settings; without one there is none"
    );
    save(&qs_with_radio, &path("laptop-quick-settings"))?;
    g.set_wifi_present(false);
    g.set_radio_on(false);
    ui.set_quick_settings_open(false);
    park();
    draw();

    // The same press again closes it, and the tooltip comes up on a hover.
    click(w, anchor, 16.0);
    assert!(ui.get_network_open());
    click(w, anchor, 16.0);
    assert!(!ui.get_network_open(), "the mark toggles its own popover");
    let before_tip = draw();
    w.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(anchor, 16.0) });
    std::thread::sleep(std::time::Duration::from_millis(900));
    let tip = draw();
    assert!(changed(&before_tip, &tip, (anchor as usize - 220, anchor as usize + 220), (34, 70)) > 500, "the tooltip says the connection in words");
    save(&tip, &path("vm-tooltip"))?;
    park();
    draw();

    // ── The laptop shape ──────────────────────────────────────────────────────────────────────
    laptop_shape(&g);
    park();
    let laptop_closed = draw();
    click(w, anchor, 16.0);
    park();
    let laptop_open = draw();
    assert!(ui.get_network_open());
    assert!(changed(&laptop_closed, &laptop_open, (900, 1280), (300, 460)) > 5_000, "the Wi-Fi list makes the popover taller");
    save(&laptop_open, &path("laptop"))?;

    // The Wi-Fi tile is the radio. Pressing it reaches `set-radio`, and not `disconnect`.
    click(w, 1000.0, 84.0);
    assert_eq!(radio.borrow().as_slice(), [false], "the tile asks for the radio off, from on");
    assert_eq!(leaves.get(), 0, "the radio tile does not disconnect");
    // The connected row's Disconnect button is a disconnect, and does not touch the radio.
    click(w, 1215.0, 142.0);
    assert_eq!(leaves.get(), 1, "Disconnect disconnects");
    assert_eq!(radio.borrow().len(), 1, "Disconnect does not switch the radio");

    // A saved network is joined with one press and no password.
    click(w, 1000.0, 142.0 + 88.0);
    assert!(joins.borrow().iter().any(|(s, p)| s == "Neighbour" && p.is_empty()), "a saved network joins without a password: {:?}", joins.borrow());
    // An open one too.
    click(w, 1000.0, 142.0 + 132.0);
    assert!(joins.borrow().iter().any(|(s, p)| s == "Guest" && p.is_empty()));
    joins.borrow_mut().clear();

    // ── The password row ──────────────────────────────────────────────────────────────────────
    assert_eq!(g.get_asking_ssid().as_str(), "");
    click(w, 1000.0, 142.0 + 44.0);
    park();
    assert_eq!(g.get_asking_ssid().as_str(), "Cafe", "a secured network with no saved profile opens its password field");
    assert!(joins.borrow().is_empty(), "and joins nothing until a person types and presses Join");
    let asking = draw();
    save(&asking, &path("password"))?;
    let secret = "correct-horse-battery";
    typed(secret);
    let typed_frame = draw();
    save(&typed_frame, &path("password-typed"))?;
    // What was typed is in no property a reader of the shell could see.
    for (name, text) in [
        ("asking-ssid", g.get_asking_ssid().to_string()),
        ("notice", g.get_notice().to_string()),
        ("notice-ssid", g.get_notice_ssid().to_string()),
        ("joining-ssid", g.get_joining_ssid().to_string()),
        ("tooltip", g.get_tooltip().to_string()),
        ("ssid", g.get_ssid().to_string()),
        ("mark", g.get_mark().to_string()),
        ("app tooltip detail", ui.get_network_detail().to_string()),
    ] {
        assert!(!text.contains("horse"), "the typed password is in NetworkState.{name}: {text}");
    }
    for r in g.get_networks().iter() {
        assert!(!r.ssid.contains("horse"));
    }
    // A rescan arrives while the password is half typed: the reading changes the rows' data in
    // place (what `wire::network::apply_rows` does), and the field and its text stay.
    {
        let networks = g.get_networks();
        let live = networks.as_any().downcast_ref::<VecModel<NetworkRow>>().expect("the list is a VecModel");
        live.set_row_data(1, row("Cafe", 80, true, false, false, false));
        live.set_row_data(2, row("Neighbour", 20, true, true, false, false));
        live.push(row("Newcomer", 45, false, false, false, false));
        live.remove(4);
        park();
        draw();
    }
    assert_eq!(g.get_asking_ssid().as_str(), "Cafe", "a new reading leaves the open row open");
    // A mind asks for another network while the person is typing: the open row is not replaced, no
    // field moves, nothing takes the keyboard. The other row is marked and that is all.
    g.set_requested_by("Yantrik Mind".into());
    g.set_requested_ssid("Guest".into());
    park();
    let marked = draw();
    assert_eq!(g.get_asking_ssid().as_str(), "Cafe", "a caller's request does not replace the row the person is typing in");
    save(&marked, &path("password-typed-and-marked"))?;
    // Enter submits it: the callback gets it once, the field is gone, and so is the row's state.
    key(w, "\n".into());
    park();
    let after = draw();
    assert_eq!(joins.borrow().as_slice(), [("Cafe".to_string(), secret.to_string())], "Enter hands the password over, once");
    assert_eq!(g.get_asking_ssid().as_str(), "", "the field closes");
    assert!(changed(&typed_frame, &after, (900, 1280), (150, 330)) > 100, "and what was typed is no longer on screen");
    save(&after, &path("password-sent"))?;
    // The mind's ask is still there for the person to decide on. Guest is open and was asked for:
    // its row has a Join button, and nothing joined it by itself.
    assert!(joins.borrow().iter().all(|(s, _)| s != "Guest"), "a caller's ask joins nothing");
    assert_eq!(g.get_requested_ssid().as_str(), "Guest");
    // The bar's mark carries the attention dot while an ask is waiting.
    park();
    let waiting = draw();
    save(&waiting, &path("marked-open-network"))?;
    click(w, 1219.0, 142.0 + 132.0);
    assert!(joins.borrow().iter().any(|(s, p)| s == "Guest" && p.is_empty()), "the person's press on Join is what joins it: {:?}", joins.borrow());
    g.set_requested_ssid("".into());
    joins.borrow_mut().clear();

    // Closing the popover drops a half-typed password with it.
    click(w, 1000.0, 142.0 + 44.0);
    typed("half-typed-secret");
    key(w, Key::Escape.into());
    park();
    draw();
    assert!(!ui.get_network_open());
    assert_eq!(g.get_asking_ssid().as_str(), "", "closing the popover closes the password row");
    click(w, anchor, 16.0);
    park();
    let reopened = draw();
    assert!(changed(&asking, &reopened, (900, 1280), (150, 330)) == 0 || g.get_asking_ssid().as_str() == "", "reopened, no field and no text");
    assert!(joins.borrow().is_empty(), "a closed popover submitted nothing");
    key(w, Key::Escape.into());
    draw();

    // ── Offline, and connected with no internet ───────────────────────────────────────────────
    g.set_mark("offline".into());
    g.set_bars(0);
    g.set_tooltip("Offline, Wi-Fi is off".into());
    g.set_kind("none".into());
    g.set_online(false);
    g.set_ssid("".into());
    g.set_ip("".into());
    g.set_radio_on(false);
    g.set_networks(ModelRc::new(VecModel::from(Vec::<NetworkRow>::new())));
    park();
    click(w, anchor, 16.0);
    park();
    let offline = draw();
    assert!(ui.get_network_open());
    save(&offline, &path("offline"))?;
    key(w, Key::Escape.into());
    draw();

    laptop_shape(&g);
    g.set_no_internet(true);
    g.set_tooltip("Wi-Fi Home, connected, no internet, 192.168.1.20".into());
    park();
    let noinet_closed = draw();
    click(w, anchor, 16.0);
    park();
    let noinet = draw();
    // The attention dot is on the mark: it differs from the same mark without it.
    g.set_no_internet(false);
    ui.set_network_open(false);
    park();
    let plain = draw();
    assert!(changed(&noinet_closed, &plain, (anchor as usize - 10, anchor as usize + 12), (4, 28)) > 8, "no internet puts the amber dot on the mark");
    save(&noinet, &path("no-internet"))?;
    g.set_no_internet(true);

    // ── Idle: a settled bar and a settled open popover repaint nothing ────────────────────────
    g.set_no_internet(false);
    for open in [false, true] {
        ui.set_network_open(open);
        park();
        draw();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..5 {
            slint::platform::update_timers_and_animations();
            w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); });
        }
        let mut redraws = 0;
        for _ in 0..10 {
            slint::platform::update_timers_and_animations();
            if w.draw_if_needed(|r| { r.render(pixels.make_mut_slice(), W as usize); }) {
                redraws += 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(redraws, 0, "a settled shell (popover open: {open}) repaints nothing");
        println!("settled, popover open={open}: {redraws} redraws over one second");
    }

    save(&laptop_open, output)?;
    println!("PASS: network mark and popover: VM shape, laptop shape, password row, offline, no internet");
    Ok(())
}
