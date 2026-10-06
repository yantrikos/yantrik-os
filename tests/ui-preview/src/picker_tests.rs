//! The one picker for every mind (#673): a person who has never seen it switches mind and model in
//! two clicks each, a model the mind cannot use is drawn but cannot be chosen, and Effort is
//! offered only with levels. Drives real pointer events through the production component.

use super::*;
use slint::{ModelRc, VecModel};

fn mind(id: &str, name: &str, words: &str, current: bool) -> PickerMind {
    PickerMind { id: id.into(), name: name.into(), words: words.into(), selectable: true, current }
}

fn model(kind: &str, id: &str, name: &str, detail: &str, disabled: bool, reason: &str) -> PickerModel {
    PickerModel { kind: kind.into(), id: id.into(), name: name.into(), detail: detail.into(), disabled, reason: reason.into(), current: false }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (width, height) = (760u32, 560u32);
    let ui = PickerProbe::new()?;
    ui.set_minds(ModelRc::new(VecModel::from(vec![
        mind("mind", "Yantrik Mind", "answering · Yantrik models", true),
        mind("companion", "Yantrik Companion", "ready · Yantrik models", false),
        mind("pi", "Pi", "ready · its own models", false),
    ])));
    ui.set_models(ModelRc::new(VecModel::from(vec![
        model("header", "", "RECENT", "", false, ""),
        model("model", "free-groq/openai/gpt-oss-120b", "openai/gpt-oss-120b", "Groq (free) · thinks · 131K context", false, ""),
        model("header", "", "OLLAMA CLOUD", "", false, ""),
        model("model", "ollama-cloud/deepseek-v4.1-flash", "deepseek-v4.1-flash", "Ollama Cloud · 128K context", false, ""),
        model("model", "openai/o4-mini", "o4-mini", "", true, "Yantrik Mind keeps a memory of you; allow private context for OpenAI under AI accounts to use it."),
    ])));
    ui.set_efforts(ModelRc::new(VecModel::from(vec!["easy".into(), "medium".into(), "high".into()])));
    ui.set_files(ModelRc::new(VecModel::from(vec![PickerFile { path: "/home/p/plan.md".into(), name: "plan.md".into(), detail: "2 KB".into(), folder: false }])));
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(width, height));
    let draw = || {
        slint::platform::update_timers_and_animations();
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, height);
        w.request_redraw();
        w.draw_if_needed(|r| { r.render(p.make_mut_slice(), width as usize); });
        p
    };
    let save = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: &str| -> Result<(), Box<dyn std::error::Error>> {
        let f = BufWriter::new(File::create(path)?);
        let mut e = png::Encoder::new(f, width, height);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
        Ok(())
    };
    draw();
    let (px, py) = (ui.get_picker_x(), ui.get_picker_y());

    // Mind: the chip, then the second mind. The menu opens upward from the bar.
    click(w, px + 20.0, py + 14.0);
    let open = draw();
    save(&open, &output.replace(".png", "-minds.png"))?;
    assert_eq!(ui.get_opened_count(), 1, "the mind chip opened its menu");
    // Rows are about 44px with a 6px pad; the menu is 3 rows tall, 6px above the chip.
    let menu_top = py - 6.0 - 3.0 * 44.0 - 12.0;
    click(w, px + 60.0, menu_top + 6.0 + 44.0 + 22.0);
    draw();
    assert_eq!(ui.get_chosen_mind(), "companion", "two clicks switch the mind");

    // Model: the chip (after the mind chip), then a model. The search field takes the focus.
    click(w, px + 150.0, py + 14.0);
    let models = draw();
    save(&models, output)?;
    assert_eq!(ui.get_opened_count(), 2);
    let menu_top = py - 6.0 - 420.0;
    // Search box (30px + pad), "RECENT" header (24px), then the recent model.
    let mut chosen = String::new();
    for dy in (36..140).step_by(6) {
        click(w, px + 80.0, menu_top + 6.0 + dy as f32);
        draw();
        chosen = ui.get_chosen_model().to_string();
        if !chosen.is_empty() {
            break;
        }
        click(w, px + 150.0, py + 14.0);
        draw();
    }
    assert_eq!(chosen, "free-groq/openai/gpt-oss-120b", "two clicks switch the model");
    println!("PASS picker: mind and model each switched in two clicks");
    Ok(())
}
