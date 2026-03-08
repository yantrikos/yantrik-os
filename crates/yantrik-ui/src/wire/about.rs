//! About screen wiring — populates system info fields once at startup.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

pub fn wire(ui: &App, _ctx: &AppContext) {
    let ui_weak = ui.as_weak();
    populate_about_info(&ui_weak);
}

fn populate_about_info(ui_weak: &slint::Weak<App>) {
    let Some(ui) = ui_weak.upgrade() else { return };

    // Hostname from /etc/hostname
    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "unknown".to_string())
        .trim()
        .to_string();
    ui.set_about_hostname(hostname.into());

    // Kernel version from /proc/version (third whitespace-delimited field)
    let kernel = std::fs::read_to_string("/proc/version")
        .ok()
        .and_then(|v| v.split_whitespace().nth(2).map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    ui.set_about_kernel(kernel.into());

    // CPU model name from /proc/cpuinfo (trimmed for display)
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| {
                    s.trim()
                        .replace("(R)", "")
                        .replace("(TM)", "")
                        .replace("  ", " ")
                        .replace(" Processor", "")
                        .trim()
                        .to_string()
                })
        })
        .unwrap_or_else(|| "unknown".to_string());
    ui.set_about_cpu(cpu.into());

    // RAM total from /proc/meminfo
    let ram = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
                .map(|kb| {
                    let gb = kb as f64 / 1024.0 / 1024.0;
                    format!("{:.1} GB", gb)
                })
        })
        .unwrap_or_else(|| "unknown".to_string());
    ui.set_about_ram(ram.into());

    // Disk info via df command
    // Try GNU df first (--output), fall back to plain df -h (BusyBox/Alpine)
    let disk = std::process::Command::new("df")
        .args(["-h", "--output=size,avail", "/"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| {
            let s = String::from_utf8_lossy(&out.stdout);
            let line = s.lines().nth(1)?;
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                Some(format!("{} free of {}", parts[1], parts[0]))
            } else {
                None
            }
        })
        .or_else(|| {
            // Fallback: plain `df -h /` — columns: Filesystem Size Used Avail Use% Mounted
            std::process::Command::new("df")
                .args(["-h", "/"])
                .output()
                .ok()
                .and_then(|out| {
                    let s = String::from_utf8_lossy(&out.stdout);
                    let line = s.lines().nth(1)?;
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 4 {
                        // parts[1]=Size, parts[3]=Avail
                        Some(format!("{} free of {}", parts[3], parts[1]))
                    } else {
                        None
                    }
                })
        })
        .unwrap_or_else(|| "unknown".to_string());
    ui.set_about_disk(disk.into());

    // Uptime from /proc/uptime
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|u| {
            u.split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|secs| {
                    let total_secs = secs as u64;
                    let days = total_secs / 86400;
                    let hours = (total_secs % 86400) / 3600;
                    let mins = (total_secs % 3600) / 60;
                    if days > 0 {
                        format!("{}d {}h {}m", days, hours, mins)
                    } else if hours > 0 {
                        format!("{}h {}m", hours, mins)
                    } else {
                        format!("{}m", mins)
                    }
                })
        })
        .unwrap_or_else(|| "\u{2014}".to_string());
    ui.set_about_uptime(uptime.into());

    // Version from Cargo.toml + git hash
    let version = match option_env!("GIT_HASH") {
        Some(hash) => format!("{} ({})", env!("CARGO_PKG_VERSION"), hash),
        None => env!("CARGO_PKG_VERSION").to_string(),
    };
    ui.set_about_version(version.into());

    // Build date from build.rs
    let build_date = option_env!("BUILD_DATE").unwrap_or("unknown");
    ui.set_about_build_date(build_date.into());
}
