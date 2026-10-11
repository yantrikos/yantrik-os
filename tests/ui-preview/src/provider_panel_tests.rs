//! Add Provider with the catalogue's 39 presets, at 1280×800. On VM 520 the grid pushed the name,
//! URL, key and Connect below the panel's bottom edge and nothing scrolled: no provider could be
//! added at all. Choosing a preset now folds the grid to one line and brings the form up.

use super::*;

fn presets() -> slint::ModelRc<ProviderPresetGroup> {
    let groups = [("CLOUD", 19), ("AGGREGATORS & BUDGET", 13), ("LOCAL INFERENCE", 7)];
    let mut n = 0;
    let out: Vec<ProviderPresetGroup> = groups
        .iter()
        .map(|(title, count)| {
            let items: Vec<ProviderPresetData> = (0..*count)
                .map(|_| {
                    n += 1;
                    ProviderPresetData {
                        id: if n == 16 { "nvidia-nim".into() } else { format!("p{n}").into() },
                        name: if n == 16 { "NVIDIA NIM".into() } else { format!("Provider {n}").into() },
                        label: if n == 16 { "NVIDIA NIM".into() } else { format!("Provider {n}").into() },
                        url: "https://example.invalid/v1".into(),
                        placeholder: "sk-…".into(),
                        group: "cloud".into(),
                    }
                })
                .collect();
            let rows: Vec<ProviderPresetRow> = items
                .chunks(5)
                .map(|c| ProviderPresetRow { items: Rc::new(slint::VecModel::from(c.to_vec())).into() })
                .collect();
            ProviderPresetGroup { title: (*title).into(), rows: Rc::new(slint::VecModel::from(rows)).into() }
        })
        .collect();
    Rc::new(slint::VecModel::from(out)).into()
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = ProviderPanelProbe::new()?;
    ui.set_presets(presets());
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(1280, 800));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1280, 800);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| { r.render(p.make_mut_slice(), 1280); });
        }
        p
    };
    let save = |p: &slint::SharedPixelBuffer<slint::Rgb8Pixel>, path: String| -> Result<(), Box<dyn std::error::Error>> {
        let f = BufWriter::new(File::create(path)?);
        let mut e = png::Encoder::new(f, 1280, 800);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(p.as_bytes())?;
        Ok(())
    };
    let stem = output.trim_end_matches(".png").to_string();
    save(&draw(), format!("{stem}-pick.png"))?;
    ui.set_preset("nvidia-nim".into());
    ui.set_name("NVIDIA NIM".into());
    save(&draw(), format!("{stem}-form.png"))?;
    println!("rendered {stem}-pick.png and {stem}-form.png");
    Ok(())
}
