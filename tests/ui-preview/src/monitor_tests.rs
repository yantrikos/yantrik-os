use super::*;

/// The System Monitor defects a person hit (issue #220): bar fills that started in the
/// middle of their track instead of the left edge, a process list that slid down under
/// the pointer the moment a row was selected, an empty filtered list that blamed the
/// machine instead of echoing the word typed, and an AI card that showed dashes for
/// measurements nothing on the box reports.
pub fn run(window: &MinimalSoftwareWindow) -> Result<(), Box<dyn std::error::Error>> {
    let ui = SystemMonitorProbe::new()?;
    // The bars start empty and the readings arrive after the window is shown, the way
    // the app's first poll delivers them. The core and disk fills report their geometry
    // through `changed` callbacks, which fire on a value changing in a laid-out tree —
    // a fill born at its final size inside a window that has not been drawn yet never
    // changes and never reports.
    ui.set_cores(slint::ModelRc::new(slint::VecModel::from(
        (0..4)
            .map(|i| CpuCoreData { core_id: i, usage: 0.0 })
            .collect::<Vec<_>>(),
    )));
    ui.set_disks(slint::ModelRc::new(slint::VecModel::from(vec![
        DiskData {
            mount_point: "/".into(),
            filesystem: "ext4".into(),
            used_bytes: "186 GB".into(),
            total_bytes: "300 GB".into(),
            usage_percent: 0.0,
        },
        DiskData {
            mount_point: "/home".into(),
            filesystem: "ext4".into(),
            used_bytes: "41 GB".into(),
            total_bytes: "100 GB".into(),
            usage_percent: 0.0,
        },
    ])));
    ui.set_procs(slint::ModelRc::new(slint::VecModel::from(vec![
        MonitorProcessData { pid: 4242, name: "fixture-shell".into(), cpu_percent: 12.5, mem_percent: 3.0, status: "Running".into() },
        MonitorProcessData { pid: 5150, name: "fixture-browser".into(), cpu_percent: 6.2, mem_percent: 21.4, status: "Sleeping".into() },
        MonitorProcessData { pid: 6001, name: "xylophoned".into(), cpu_percent: 1.0, mem_percent: 0.5, status: "Running".into() },
    ])));
    ui.show()?;
    window.set_size(slint::PhysicalSize::new(1280, 800));
    // The bar fills animate their width over 300ms, so every geometry check below runs
    // after a settle that outlasts the animation instead of racing it.
    let settle = || {
        for _ in 0..10 {
            slint::platform::update_timers_and_animations();
            let mut p = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(1280, 800);
            window.request_redraw();
            window.draw_if_needed(|r| { r.render(p.make_mut_slice(), 1280); });
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
    };
    settle();
    // The first poll lands. Usages stay off 0 and 100 on purpose: an empty or a full
    // bar is also "centred" at x=0, so it could not tell the fix from the defect.
    let cores = ui.get_cores();
    for (i, usage) in [62.0f32, 45.0, 73.0, 38.0].iter().enumerate() {
        slint::Model::set_row_data(&cores, i, CpuCoreData { core_id: i as i32, usage: *usage });
    }
    let disks = ui.get_disks();
    for (i, usage) in [62.0f32, 41.0].iter().enumerate() {
        let mut disk = slint::Model::row_data(&disks, i).unwrap();
        disk.usage_percent = *usage;
        slint::Model::set_row_data(&disks, i, disk);
    }
    settle();
    let mut problems: Vec<String> = vec![];

    // ── Fills start at the left edge of their track ──
    //
    // A Rectangle inside another Rectangle that sets a width but no x is centred by the
    // Slint compiler, so 62% used to be drawn from about 20% of the track. Every fill in
    // the screen now pins x: 0. The core and disk fills live inside repeaters, so the
    // screen mirrors the last one that moved; with no core or disk at 0 or 100, any
    // centring at all shows up as a non-zero x here.
    if ui.get_swap_fill_width() <= 0.0 {
        problems.push("the fixture never reached the swap bar: its fill has zero width".into());
    }
    if ui.get_core_fill_width() <= 0.0 {
        problems.push("the fixture never reached the core bars: their fill has zero width".into());
    }
    if ui.get_disk_fill_width() <= 0.0 {
        problems.push("the fixture never reached the disk bars: their fill has zero width".into());
    }
    if ui.get_health_fill_x() != 0.0 {
        problems.push(format!("health bar fill starts {}px inside its track instead of at the left edge", ui.get_health_fill_x()));
    }
    if ui.get_swap_fill_x() != 0.0 {
        problems.push(format!("swap bar fill starts {}px inside its track instead of at the left edge", ui.get_swap_fill_x()));
    }
    if ui.get_core_fill_x() != 0.0 {
        problems.push(format!("a per-core bar fill starts {}px inside its track instead of at the left edge", ui.get_core_fill_x()));
    }
    if ui.get_disk_fill_x() != 0.0 {
        problems.push(format!("a disk bar fill starts {}px inside its track instead of at the left edge", ui.get_disk_fill_x()));
    }

    // ── Selecting a row does not move the list ──
    //
    // The PROCESSES header used to grow 14px the moment a selection produced the End
    // and Force Kill buttons, sliding the search bar and every row down under the
    // pointer. The header now reserves its button row's height whether or not anything
    // is selected, so the search bar's y must not budge across a selection.
    let header_idle = ui.get_proc_header_height();
    let search_idle = ui.get_proc_search_y();
    ui.set_selected_pid(5150);
    settle();
    if ui.get_proc_header_height() != header_idle {
        problems.push(format!(
            "selecting a process grew the header from {}px to {}px, pushing the list down under the pointer",
            header_idle,
            ui.get_proc_header_height()
        ));
    }
    if ui.get_proc_search_y() != search_idle {
        problems.push(format!(
            "selecting a process moved the process list: the search bar was at y={} and is now at y={}",
            search_idle,
            ui.get_proc_search_y()
        ));
    }
    ui.set_selected_pid(-1);
    settle();

    // ── An empty filtered list echoes the word typed ──
    ui.set_procs(slint::ModelRc::new(slint::VecModel::from(vec![])));
    ui.set_search("xylo".into());
    settle();
    if ui.get_process_empty_text() != "No process matches 'xylo'" {
        problems.push(format!(
            "a filter that matched nothing says \"{}\" instead of naming the word typed",
            ui.get_process_empty_text()
        ));
    }
    ui.set_search("".into());
    settle();
    if ui.get_process_empty_text() != "No process data" {
        problems.push(format!(
            "with no filter and no processes the list says \"{}\" instead of \"No process data\"",
            ui.get_process_empty_text()
        ));
    }

    // ── The AI card is honest about having no source ──
    //
    // Nothing on this machine feeds the model or token inputs, so the card used to sit
    // full of dashes. It now says plainly that nothing is measured, and only falls back
    // to numbers when a real source reports some.
    if ui.get_ai_no_source_text().is_empty() {
        problems.push("the AI workloads card claims measurements while nothing feeds its inputs".into());
    }
    ui.set_ai_model_name("fixture-model".into());
    settle();
    if !ui.get_ai_no_source_text().is_empty() {
        problems.push("the AI workloads card still says nothing is measured while a model name is reporting".into());
    }

    assert!(problems.is_empty(), "System Monitor problems:\n{}", problems.join("\n"));
    println!("PASS: System Monitor bar fills start at the left edge, selecting a process does not move the list, an empty filter echoes the word typed, and the AI card says when it has no source");
    Ok(())
}
