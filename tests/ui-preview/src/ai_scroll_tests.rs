//! Settings → AI & Intelligence must scroll (#998). On the 520 gate, six wheel clicks moved the
//! Appearance page 360 px and about sixty clicks at five places on the AI page moved nothing.
//! This draws the page at the size of a real window (1280x800) with realistic data (the map, three
//! accounts, three saved providers, the free tiers), reads back the page's scroll geometry, and
//! checks that the content is far taller than the window, that the wheel moves it, and that the
//! last section is reachable.

use super::*;
use slint::{ComponentHandle, ModelRc, VecModel};

const W: u32 = 1280;
const H: u32 = 800;

fn mind(name: &str, state: &str, runs_on: &str) -> RunsOnRow {
    RunsOnRow {
        name: name.into(),
        state: state.into(),
        answering: state == "Answering",
        runs_on: runs_on.into(),
        source: "Set in its own settings; this is what it says".into(),
        details: format!("{name}: {runs_on}").into(),
    }
}

fn account(id: &str, name: &str, local: bool, sharing: &str) -> AiAccountRow {
    AiAccountRow {
        id: id.into(),
        name: name.into(),
        summary: "12 models · 3 think · 2 see images".into(),
        state: "ready".into(),
        detail: format!("Listed by {name} itself.").into(),
        local,
        sharing: sharing.into(),
        share_words: format!("Let your Minds that use Yantrik models share what they know about you with {name}").into(),
        private_words: if local { "On this computer or your network.".into() } else { format!("Requests from your Minds that use Yantrik models to {name} are not sent.").into() },
        switchable: !local,
        offers_share: !local,
        lane_model: if local { format!("{id}/qwen3:8b").into() } else { "".into() },
        is_lane: false,
    }
}

fn provider(id: &str, name: &str, primary: bool) -> AIProviderData {
    AIProviderData {
        id: id.into(),
        name: name.into(),
        provider_type: "openai".into(),
        base_url: "https://api.example.com/v1".into(),
        status: "connected".into(),
        is_primary: primary,
        is_fallback: false,
        latency_ms: -1,
        error_message: "".into(),
    }
}

fn free(id: &str, name: &str, state: &str, trains: bool) -> FreeAiRow {
    FreeAiRow {
        id: id.into(),
        name: name.into(),
        state: state.into(),
        state_words: state.into(),
        offer: "1,000 requests and 200K tokens a day per model.".into(),
        needs: "Needs: an account · about 3 min".into(),
        detail: "".into(),
        trains,
        on: true,
        values: ModelRc::new(VecModel::from(vec![])),
        opt_out: "".into(),
    }
}

pub fn run(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = AiScrollProbe::new()?;
    ui.set_category(0);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| {
                r.render(p.make_mut_slice(), W as usize);
            });
        }
        p
    };
    // As in the shell: the person opens Settings on Appearance, goes to AI & Intelligence, and the
    // services fill the page afterwards.
    draw();
    ui.set_category(1);
    let empty = draw();
    println!("AI page, before any data: visible {}px, content {}px", ui.get_visible_height(), ui.get_viewport_height());
    ui.set_rows(ModelRc::new(VecModel::from(vec![
        mind("Yantrik Mind", "Answering", "Ollama Cloud \u{b7} deepseek-v4.1-flash"),
        mind("Yantrik Companion", "Built in", "A service you set up \u{b7} qwen3.8:27b"),
        mind("DeepSeek", "Attached", "Ollama Cloud \u{b7} deepseek-v4.1-flash"),
        mind("OpenClaw", "Attached", "Ollama Cloud \u{b7} kimi-k3"),
        mind("Pi", "Attached", "Ollama Cloud \u{b7} deepseek-v4.1-flash"),
        mind("Hermes Agent", "Attached", "No model picked yet"),
    ])));
    ui.set_summary("Ollama Cloud runs 4 minds. A service you set up runs 1 mind.".into());
    ui.set_providers(ModelRc::new(VecModel::from(vec![
        provider("p1", "OpenAI", true),
        provider("p2", "Ollama Cloud", false),
        provider("p3", "NVIDIA NIM", false),
    ])));
    ui.set_status(AIStatusData {
        provider_name: "OpenAI".into(),
        provider_type: "from Settings".into(),
        model_name: "gpt-4o".into(),
        model_tier: "".into(),
        status: "connected".into(),
        detail: "".into(),
    });
    let accounts = ui.global::<AiAccountsState>();
    accounts.set_rows(ModelRc::new(VecModel::from(vec![
        account("openai", "OpenAI", false, "off"),
        account("ollama-cloud", "Ollama Cloud", false, "lane"),
        account("ollama", "Ollama", true, "share"),
    ])));
    accounts.set_summary("3 accounts · 36 models".into());
    let tiers = ui.global::<FreeAiState>();
    let rows = vec![
        free("groq", "Groq", "ready", false),
        free("cloudflare", "Cloudflare Workers AI", "not-set-up", false),
        free("openrouter", "OpenRouter", "not-set-up", true),
        free("gemini", "Google Gemini", "not-set-up", true),
    ];
    let (trains, clean): (Vec<FreeAiRow>, Vec<FreeAiRow>) = rows.iter().cloned().partition(|r| r.trains);
    tiers.set_clean_rows(ModelRc::new(VecModel::from(clean)));
    tiers.set_trains_rows(ModelRc::new(VecModel::from(trains)));
    tiers.set_rows(ModelRc::new(VecModel::from(rows)));
    tiers.set_summary("1 of 4 ready".into());
    tiers.set_expanded(true);

    let top = draw();
    let _ = empty;
    let (visible, viewport) = (ui.get_visible_height(), ui.get_viewport_height());
    println!("AI page: visible {visible}px, content {viewport}px");

    // The page is a long list of sections: its content must be far taller than the window.
    assert!(visible > 300.0 && visible <= H as f32, "the page's Flickable is {visible}px tall in a {H}px window");
    assert!(
        viewport > visible + 500.0,
        "the AI page's content is {viewport}px for a {visible}px window: it cannot scroll (#998)"
    );

    // The wheel moves it, all the way down.
    let at = slint::LogicalPosition::new(W as f32 - 200.0, H as f32 / 2.0);
    let mut last = 0.0;
    for _ in 0..40 {
        w.dispatch_event(slint::platform::WindowEvent::PointerScrolled { position: at, delta_x: 0.0, delta_y: -400.0 });
        std::thread::sleep(std::time::Duration::from_millis(60));
        draw();
        let y = ui.get_viewport_y();
        if (y - last).abs() < 0.5 {
            break;
        }
        last = y;
    }
    std::thread::sleep(std::time::Duration::from_millis(350));
    let bottom = draw();
    let y = ui.get_viewport_y();
    println!("scrolled to viewport-y {y}px of {}px", -(viewport - visible));
    assert!(y < -500.0, "sixty wheel clicks moved the AI page only {y}px");
    // At the end, the last section (the companion's links) is what the window shows.
    assert!(y <= -(viewport - visible) + 2.0, "the bottom of the page is not reachable: viewport-y {y}, needs {}", -(viewport - visible));
    let changed = (0..(W * H) as usize).filter(|&i| top.as_slice()[i] != bottom.as_slice()[i]).count();
    assert!(changed > 200_000, "scrolling to the bottom changed only {changed} pixels");

    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, W, H);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(bottom.as_bytes())?;
    println!("PASS ai scroll: {viewport}px of content in a {visible}px window, wheel reaches the bottom ({changed} pixels changed)");
    Ok(())
}

/// The same page inside the whole shell (app.slint's App): the window frame around it, the
/// properties the shell binds, a maximized Settings window.
pub fn run_in_shell(w: &MinimalSoftwareWindow, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ui = App::new()?;
    ui.set_current_screen(7);
    ui.set_settings_category(0);
    ui.show()?;
    w.set_size(slint::PhysicalSize::new(W, H));
    let draw = || {
        let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(W, H);
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            w.request_redraw();
            w.draw_if_needed(|r| {
                r.render(p.make_mut_slice(), W as usize);
            });
        }
        p
    };
    draw();
    let at = slint::LogicalPosition::new(760.0, 450.0);
    let wheel = |clicks: u32| {
        for _ in 0..clicks {
            w.dispatch_event(slint::platform::WindowEvent::PointerScrolled { position: at, delta_x: 0.0, delta_y: -60.0 });
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    };
    // Appearance first: the wheel must move it, or the harness is not delivering wheel events.
    let appearance = draw();
    wheel(6);
    let after_appearance = draw();
    let appearance_changed = (0..(W * H) as usize).filter(|&i| appearance.as_slice()[i] != after_appearance.as_slice()[i]).count();
    println!("shell: Appearance after 6 clicks: {appearance_changed} pixels changed");

    ui.set_settings_category(1);
    ui.set_runs_on_rows(ModelRc::new(VecModel::from(vec![
        mind("Yantrik Mind", "Answering", "Ollama Cloud, deepseek-v4.1-flash"),
        mind("Yantrik Companion", "Built in", "A service you set up, qwen3.8:27b"),
    ])));
    ui.set_runs_on_summary("Ollama Cloud runs 1 mind.".into());
    ui.set_settings_ai_providers(ModelRc::new(VecModel::from(vec![provider("p1", "OpenAI", true)])));
    let before = draw();
    wheel(30);
    let after = draw();
    let changed = (0..(W * H) as usize).filter(|&i| before.as_slice()[i] != after.as_slice()[i]).count();
    let f = BufWriter::new(File::create(output)?);
    let mut e = png::Encoder::new(f, W, H);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(after.as_bytes())?;
    println!("shell: AI page after 30 clicks: {changed} pixels changed");
    assert!(changed > 100_000, "the wheel did not move the AI page inside the shell ({changed} pixels changed)");
    Ok(())
}
