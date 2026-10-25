//! The Mind's view of System Monitor: a projection of the committed model, in the same keys,
//! values and words the app has always answered, so `yos check system-monitor` reads the same.
//!
//! This window holds, in numbers, everything the companion previously had to shell out for:
//! `top`, `df`, `free`, `uptime`. It refreshes every two seconds anyway (every thirty while the
//! window is off screen), so reading it costs nothing and is about as current as a subprocess.
//!
//! The figures are the reading's, to a tenth, in the window's `f32`. The window draws a
//! process's share to the same tenth, and a card's percentage to the whole number beside it
//! (`crate::figures`), so the two never disagree about what is shown.

use serde_json::{json, Value};
use yantrik_app::control::View;

use crate::model::Model;
use crate::outcome::{self, Provenance};
use crate::report;

/// How many of the busiest processes `describe` names. The window shows a hundred; a caller
/// asking "what is eating the machine" wants the top of the list, not a transcript of it.
const TOP: usize = 10;

/// One decimal, in the window's `f32`.
fn tenth(value: f32) -> f32 {
    (value * 10.0).round() / 10.0
}

pub fn view(m: &Model) -> View {
    let r = &m.readout;
    let top: Vec<Value> = m
        .shown()
        .iter()
        .take(TOP)
        .map(|p| {
            json!({
                "pid": p.pid as i32,
                "name": p.name,
                "cpu_percent": tenth(p.cpu_percent as f32),
                "mem_percent": tenth(p.mem_percent as f32),
                "status": p.state,
            })
        })
        .collect();
    let disks: Vec<Value> = r
        .disks
        .iter()
        .map(|d| {
            json!({
                "mount": d.mount_point,
                "used": d.used,
                "total": d.total,
                "percent": tenth(d.usage_percent),
            })
        })
        .collect();
    let interfaces: Vec<Value> = r
        .interfaces
        .iter()
        .map(|n| {
            json!({
                "name": n.name,
                // Null, not "". Neither the service's snapshot nor `sysinfo` carries an address —
                // `NetworkInterface` in the contract has no field for one — so this is a thing
                // nobody has measured, not an interface with no address.
                "ip": outcome::measured(""),
            })
        })
        .collect();

    let reading = m.reading.clone().unwrap_or_else(Provenance::service);

    // The row's own words first — the same sentence the person is reading at the top of the
    // window — then what the row leaves to the cards.
    let summary = report::summary(&m.status, &r.memory_used, &r.memory_total, &r.uptime);

    report::with_status(View::new(summary), &m.status)
        // Where every number above came from. A caller acting on these readings is entitled to
        // know that the service answered them, because when it did not, the fallback's blind
        // spots (the CPU model, the interface addresses) used to arrive looking exactly like
        // measurements.
        .with("source", reading.source.as_str())
        .with("degraded", reading.degraded())
        .with(
            "degraded_reason",
            reading.reason.as_deref().map(Value::from).unwrap_or(Value::Null),
        )
        // Said twice: this is the same text the person is looking at on the strip.
        .with("notice", m.notice())
        // Null, with `cpu_window_ms`, until there is an interval to measure over.
        .with("cpu_percent", report::cpu_percent(r.cpu_usage, m.cpu_window_ms))
        .with("cpu_window_ms", m.cpu_window_ms)
        // Null when nothing has measured it, which today is always: the snapshot contract has
        // no model string in it, so neither path can fill this in. It was reported as `""`, and
        // an audit read that as a CPU whose model is empty.
        .with("cpu_model", outcome::measured(""))
        .with("load_average", json!([r.load_avg[0], r.load_avg[1], r.load_avg[2]]))
        .with("memory_percent", (r.memory_usage * 10.0).round() as f64 / 10.0)
        .with("memory_used", r.memory_used.clone())
        .with("memory_total", r.memory_total.clone())
        .with("memory_available", r.memory_available.clone())
        .with("swap_percent", (r.swap_usage * 10.0).round() as f64 / 10.0)
        .with("uptime", r.uptime.clone())
        // What the search box holds: `top_processes` is the top of the FILTERED list.
        .with("process_filter", m.search.clone())
        .with("top_processes", Value::Array(top))
        .with("disks", Value::Array(disks))
        .with("interfaces", Value::Array(interfaces))
}
