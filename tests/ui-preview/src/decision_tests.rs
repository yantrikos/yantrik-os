use super::*;
use slint::{ModelRc, VecModel};

const PRESETS: &[(&str, &str)] = &[
    ("off", "Off: no decision model"),
    ("kev", "Kev (on this machine or the home GPU box)"),
    ("laya", "Laya (small, runs on this machine's CPU)"),
    ("jeff", "Jeff (on this machine)"),
    ("jev", "Jev (TypeSafe cloud: what is judged leaves this machine)"),
    ("systemone", "Another System One server"),
    ("chat_model", "The chat model (slower, uncalibrated)"),
];

const USES: &[(&str, &str, &str, bool)] = &[
    ("route_tools", "Choose the tool a request needs", "your request and your last few messages (never the assistant's replies)", true),
    ("browser_commitment", "Spot purchases, sends and deletes in the browser (adds a card, never removes one)", "the button, its page's title and address, and the text around it", true),
    ("agent", "Answer agents' quick questions, such as the Mind's (only a model on this machine or your home network)", "what the agent asks about", false),
];

fn save(pixels: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let f = BufWriter::new(File::create(path)?);
    let mut e = png::Encoder::new(f, width, height);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(pixels.as_bytes())?;
    Ok(())
}

pub fn run(w: &MinimalSoftwareWindow, output: &str, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
    let ui = DecisionProbe::new()?;
    ui.set_canvas_width(width as f32);
    ui.set_canvas_height(height as f32);
    let presets: Vec<DecisionPreset> = PRESETS.iter().map(|(id, label)| DecisionPreset { id: (*id).into(), label: (*label).into() }).collect();
    ui.set_presets(ModelRc::new(VecModel::from(presets)));
    let uses: Vec<DecisionUse> = USES
        .iter()
        .map(|(id, label, sends, on)| DecisionUse { id: (*id).into(), label: (*label).into(), sends: (*sends).into(), on: *on })
        .collect();
    ui.set_uses(ModelRc::new(VecModel::from(uses)));
    ui.set_where_note("Runs on a machine on your home network.".into());
    ui.set_status("Answered in 41 ms (kev kev-latest): \"Place your order\" is a commitment, p = 0.93.".into());
    ui.set_status_good(true);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width as usize); });
        p
    };
    let kev = draw();
    save(&kev, output, width, height)?;

    // Every use has its own switch: pressing one reports that use's id. The switches sit at the
    // card's right edge, below the presets.
    let mut switched = String::new();
    'scan: for y in (200..height as i32).step_by(6) {
        for x in [width as f32 - 60., width as f32 - 75., width as f32 - 90.] {
            click(w, x, y as f32);
            let a = ui.get_action().to_string();
            if a.starts_with("use:") {
                switched = a;
                break 'scan;
            }
        }
    }
    assert!(USES.iter().any(|(id, ..)| switched == format!("use:{id}")), "no use's switch answered a click: {switched:?}");

    // Jev: the note that what is judged leaves the machine.
    ui.set_provider("jev".into());
    ui.set_endpoint("https://api.typesafe.ai".into());
    ui.set_model("jev-latest".into());
    ui.set_where_note("Runs in the cloud: what it judges (a request, a button and its page) is sent there.".into());
    ui.set_leaves(true);
    ui.set_status("".into());
    let jev = draw();
    save(&jev, &output.replace(".png", "-jev.png"), width, height)?;
    assert_ne!(kev.as_bytes(), jev.as_bytes(), "switching the provider redraws the card");

    // Off: no server fields, no uses.
    ui.set_provider("off".into());
    ui.set_where_note("".into());
    ui.set_leaves(false);
    let off = draw();
    save(&off, &output.replace(".png", "-off.png"), width, height)?;
    assert_ne!(off.as_bytes(), jev.as_bytes());

    // A preset row is pressed: its id reaches the callback. Rows sit under the description;
    // scan down the left column until one answers.
    let mut chosen = String::new();
    for y in (90..360).step_by(6) {
        click(w, 200., y as f32);
        let a = ui.get_action().to_string();
        if a.starts_with("choose:") {
            chosen = a;
            break;
        }
    }
    assert!(chosen.starts_with("choose:"), "no preset row answered a click");
    println!("PASS: Decision model card renders Kev (home), Jev (cloud note) and Off, one switch per use ({switched}), and a preset row reports {chosen} at {width}x{height}");
    Ok(())
}
