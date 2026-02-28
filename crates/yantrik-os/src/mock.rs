//! Mock observer — fake system events on timers.
//!
//! Used for QEMU development where D-Bus services (UPower, NetworkManager)
//! don't exist. Simulates a realistic sequence of system events.

use crossbeam_channel::Sender;
use std::io::Write;
use std::time::Duration;

use crate::events::{FileChangeKind, SystemEvent};

/// Run the mock observer. Emits fake events in a cycle.
pub fn run_mock_observer(tx: Sender<SystemEvent>) {
    tracing::info!("Mock observer running — fake events every few seconds");

    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let cmd_log_path = format!("{}/.yantrik/cmd_log", home);

    let mut tick: u64 = 0;
    let mut battery_level: u8 = 85;
    let mut battery_charging = false;
    let mut network_connected = true;

    loop {
        std::thread::sleep(Duration::from_secs(5));
        tick += 1;

        // Battery drains slowly (every 30s = 6 ticks)
        if tick % 6 == 0 {
            if battery_charging {
                battery_level = (battery_level + 2).min(100);
                if battery_level >= 100 {
                    battery_charging = false;
                }
            } else {
                battery_level = battery_level.saturating_sub(1);
                // Start charging at 10%
                if battery_level <= 10 {
                    battery_charging = true;
                }
            }

            let _ = tx.send(SystemEvent::BatteryChanged {
                level: battery_level,
                charging: battery_charging,
                time_to_empty_mins: if battery_charging {
                    None
                } else {
                    Some(battery_level as u32 * 3) // rough estimate
                },
            });
        }

        // Network toggles every 2 minutes (24 ticks)
        if tick % 24 == 0 {
            network_connected = !network_connected;
            let _ = tx.send(SystemEvent::NetworkChanged {
                connected: network_connected,
                ssid: if network_connected {
                    Some("YantrikNet".to_string())
                } else {
                    None
                },
                signal: if network_connected { Some(72) } else { None },
            });
        }

        // CPU pressure every 10s (2 ticks) — varies randomly-ish
        if tick % 2 == 0 {
            let cpu = 15.0 + ((tick as f32 * 1.7).sin() * 20.0).abs();
            let _ = tx.send(SystemEvent::CpuPressure {
                usage_percent: cpu,
            });
        }

        // Memory pressure every 10s
        if tick % 2 == 0 {
            // Simulate ~400MB used out of 1GB
            let base = 400_000_000u64;
            let jitter = ((tick as f64 * 0.3).sin() * 50_000_000.0) as u64;
            let _ = tx.send(SystemEvent::MemoryPressure {
                used_bytes: base + jitter,
                total_bytes: 1_000_000_000,
            });
        }

        // Fake file events every 45s (9 ticks)
        if tick % 9 == 0 {
            let files = [
                ("~/Downloads/report.pdf", FileChangeKind::Created),
                ("~/Downloads/photo.jpg", FileChangeKind::Created),
                ("~/Documents/notes.txt", FileChangeKind::Modified),
            ];
            let idx = (tick / 9) as usize % files.len();
            let (path, kind) = &files[idx];
            let _ = tx.send(SystemEvent::FileChanged {
                path: path.to_string(),
                kind: kind.clone(),
            });
        }

        // Fake shell error every 90s (18 ticks) — for ErrorCompanion
        if tick % 18 == 0 {
            let mock_cmds = [
                "cargo build",
                "git push origin main",
                "npm install",
                "python3 train.py",
                "make test",
            ];
            let cmd = mock_cmds[(tick / 18) as usize % mock_cmds.len()];
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let line = format!("{}\t{}\t1\n", ts, cmd);
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&cmd_log_path)
            {
                let _ = f.write_all(line.as_bytes());
            }
            let _ = tx.send(SystemEvent::FileChanged {
                path: cmd_log_path.clone(),
                kind: FileChangeKind::Modified,
            });
        }

        // Fake notifications every 35s (7 ticks)
        if tick % 7 == 0 {
            let notifications = [
                ("Firefox", "Download Complete", "report.pdf has finished downloading", 1u8),
                ("Thunderbird", "New Email", "Meeting tomorrow at 10am — from Alice", 1),
                ("System", "Update Available", "3 packages can be upgraded", 0),
                ("Signal", "New Message", "Hey, are you free this afternoon?", 1),
                ("Disk Monitor", "Low Disk Space", "/home is 92% full", 2),
            ];
            let idx = (tick / 7) as usize % notifications.len();
            let (app, summary, body, urgency) = notifications[idx];
            let _ = tx.send(SystemEvent::NotificationReceived {
                app: app.to_string(),
                summary: summary.to_string(),
                body: body.to_string(),
                urgency,
            });
        }

        // Fake process start/stop every 60s (12 ticks)
        if tick % 12 == 0 {
            let cycle = (tick / 12) % 2;
            if cycle == 0 {
                let _ = tx.send(SystemEvent::ProcessStarted {
                    name: "firefox-esr".to_string(),
                    pid: 1234 + (tick as u32 % 100),
                    cpu_percent: 8.5,
                });
            } else {
                let _ = tx.send(SystemEvent::ProcessStopped {
                    name: "firefox-esr".to_string(),
                    pid: 1234 + ((tick - 12) as u32 % 100),
                    exit_code: Some(0),
                });
            }
        }
    }
}
