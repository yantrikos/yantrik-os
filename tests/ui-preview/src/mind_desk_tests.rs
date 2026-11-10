use super::*;

/// The Mind desk's strips (#1023, phase 1), drawn around a stand-in for Mind View's apps in five
/// moments of the approved design: a Mind at work, one that needs the person, one that failed, a
/// Mind that finished after a stop the person allowed, and a shell that is not answering. The words are
/// fixtures: what a harness would say, and what the desk would make of it
/// (apps/mind-desk/src/words.rs holds the real wording, and its own tests hold it).
///
/// Checked: the three strips are the height the compositor's margin reserves (160, and the pixel
/// over the apps), the apps' area starts under them, and every scene drew. One still per moment, and a
/// collage of all five.
pub fn run(window: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    const W: u32 = 1184;
    const H: u32 = 660;
    let ui = MindDeskProbe::new()?;
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(W, H));
    let mut problems: Vec<String> = Vec::new();
    let mut stills: Vec<(String, slint::SharedPixelBuffer<slint::Rgb8Pixel>)> = Vec::new();

    for scene in scenes() {
        apply(&ui, &scene);
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..4 {
            slint::platform::update_timers_and_animations();
            pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
            window.request_redraw();
            window.draw_if_needed(|r| {
                r.render(pixels.make_mut_slice(), W as usize);
            });
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let path = output.replace(".png", &format!("-{}.png", scene.name));
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path)?), W, H);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(pixels.as_bytes())?;
        println!("{}: drawn to {path}", scene.name);
        stills.push((scene.name.to_string(), pixels));
    }

    // The geometry the compositor reserves: the margin is the three strips, the window a pixel more.
    if ui.get_header_height() + ui.get_strip_height() + ui.get_footer_height() != 161.0 {
        problems.push("the bar is not 161px: 160 reserved and the pixel over the apps".to_string());
    }
    if ui.get_middle_top() != 161.0 {
        problems.push("the apps' area does not start under the bar".to_string());
    }
    // Five scenes that are the same picture are five scenes that did not change.
    for pair in stills.windows(2) {
        if pair[0].1.as_bytes() == pair[1].1.as_bytes() {
            problems.push(format!("{} and {} drew the same picture", pair[0].0, pair[1].0));
        }
    }

    // The collage: the bar of each scene, one under the other.
    let bar_h = 161u32;
    let mut collage = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, bar_h * stills.len() as u32);
    for (i, (_, still)) in stills.iter().enumerate() {
        let src = still.as_bytes();
        let dst = collage.make_mut_slice();
        let row = W as usize;
        for y in 0..bar_h as usize {
            let to = i * bar_h as usize + y;
            for x in 0..row {
                dst[to * row + x] = slint::Rgb8Pixel { r: src[(y * row + x) * 3], g: src[(y * row + x) * 3 + 1], b: src[(y * row + x) * 3 + 2] };
            }
        }
    }
    let path = output.replace(".png", "-collage.png");
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path)?), W, bar_h * stills.len() as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(collage.as_bytes())?;

    assert!(problems.is_empty(), "the Mind desk:\n{}", problems.join("\n"));
    println!("PASS: the Mind desk draws its bar at the height the compositor reserves, in five moments");
    Ok(())
}

struct Scene {
    name: &'static str,
    label: &'static str,
    tone: i32,
    headline: &'static str,
    can_stop: bool,
    stop_note: &'static str,
    agents: bool,
    heading: &'static str,
    tool: &'static str,
    mark: &'static str,
    stats: &'static str,
    apps: &'static str,
    safety: &'static str,
    model: &'static str,
    model_mark: &'static str,
}

const PERSON: &str = "Runs as you · OS actions ask first · Not isolated (planned)";
const OWN: &str = "Own account · Network checked · OS actions ask first";

fn scenes() -> Vec<Scene> {
    let working = Scene {
        name: "working",
        label: "HERMES · RUNNING A TOOL",
        tone: 1,
        headline: "Working on it",
        can_stop: true,
        stop_note: "",
        agents: true,
        heading: "RUNNING TOOL",
        tool: "blender.add_object",
        mark: "◇ Reported · Running",
        stats: "4:32 elapsed at 14:32 · 12 calls · 1 failed · 1 interrupted",
        apps: "Blender / Terminal / Files",
        safety: PERSON,
        model: "deepseek-v4.1-flash via ollama-cloud",
        model_mark: "◇ Harness-reported model",
    };
    let waiting = Scene {
        name: "waiting",
        label: "HERMES · WAITING FOR YOU",
        tone: 2,
        headline: "Waiting for you",
        can_stop: false,
        stop_note: "Waiting for your answer on the desktop",
        heading: "LAST TOOL",
        tool: "files.write",
        mark: "◆ Verified · Ok",
        ..working_copy()
    };
    let failed = Scene {
        name: "failed",
        label: "HERMES · FAILED",
        tone: 3,
        headline: "Failed",
        can_stop: false,
        heading: "LAST TOOL",
        tool: "agent_run",
        mark: "◆ Verified · Failed",
        stats: "6:05 elapsed at 14:38 · 17 calls · 2 failed · 1 interrupted",
        ..working_copy()
    };
    let done = Scene {
        name: "done-stopped",
        label: "YANTRIK MIND · DONE",
        tone: 4,
        headline: "Done",
        can_stop: false,
        stop_note: "Stopped",
        heading: "LAST TOOL",
        tool: "notes.save",
        mark: "◆ Verified · Ok",
        stats: "1:12 elapsed at 14:40 · 4 calls · 0 failed · 0 interrupted",
        apps: "Notes",
        safety: OWN,
        model: "",
        model_mark: "",
        ..working_copy()
    };
    let lost = Scene {
        name: "not-connected",
        label: "MIND VIEW · NOT CONNECTED",
        tone: 0,
        headline: "The shell is not answering",
        can_stop: false,
        agents: false,
        heading: "",
        tool: "",
        mark: "",
        stats: "",
        apps: "Apps unavailable",
        safety: "Account unavailable",
        model: "",
        model_mark: "",
        ..working_copy()
    };
    vec![working, waiting, failed, done, lost]
}

/// The base the other moments change.
fn working_copy() -> Scene {
    Scene {
        name: "",
        label: "",
        tone: 0,
        headline: "",
        can_stop: true,
        stop_note: "",
        agents: true,
        heading: "",
        tool: "",
        mark: "",
        stats: "4:32 elapsed at 14:36 · 13 calls · 1 failed · 1 interrupted",
        apps: "Blender / Terminal / Files",
        safety: PERSON,
        model: "deepseek-v4.1-flash via ollama-cloud",
        model_mark: "◇ Harness-reported model",
    }
}

fn apply(ui: &MindDeskProbe, s: &Scene) {
    ui.set_label(s.label.into());
    ui.set_tone(s.tone);
    ui.set_headline(s.headline.into());
    ui.set_can_stop(s.can_stop);
    ui.set_stop_note(s.stop_note.into());
    ui.set_agents_offered(s.agents);
    ui.set_strip_heading(s.heading.into());
    ui.set_strip_tool(s.tool.into());
    ui.set_strip_mark(s.mark.into());
    ui.set_strip_stats(s.stats.into());
    ui.set_apps(s.apps.into());
    ui.set_safety(s.safety.into());
    ui.set_model(s.model.into());
    ui.set_model_mark(s.model_mark.into());
}
