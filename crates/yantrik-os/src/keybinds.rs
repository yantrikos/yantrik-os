//! Global keyboard shortcuts — D-Bus service + labwc rc.xml generator.
//!
//! labwc keybinds execute `dbus-send` to org.yantrik.Keybinds.Trigger,
//! which emits KeybindTriggered system events for the UI to handle.
//!
//! Default bindings:
//! - Super       → open-lens (Intent Lens)
//! - Super+L     → lock-screen
//! - Super+T     → open-terminal
//! - Super+E     → open-files
//! - Super+S     → open-settings
//! - Print       → screenshot (full screen, save to file)
//! - Shift+Print → screenshot-region (select region, save to file)
//! - Ctrl+Print  → screenshot-clipboard (full screen, copy to clipboard)
//! - Ctrl+Shift+Print → screenshot-clipboard-region (select region, copy to clipboard)
//! - Super+V     → clipboard-history (clipboard history panel)
//! - Super+D     → toggle-dnd (Do Not Disturb)
//! - Super+Space → open-lens (alternative)
//! - Super+Left  → snap window to left half
//! - Super+Right → snap window to right half
//! - Super+Up    → maximize window
//! - Super+Down  → unsnap / restore window

use crossbeam_channel::Sender;

use crate::SystemEvent;

/// A single keybinding definition.
pub struct Keybind {
    /// labwc key spec, e.g. "Super_L", "W-t", "Print".
    pub key: &'static str,
    /// Action identifier sent via D-Bus.
    pub action: &'static str,
    /// Human-readable description.
    pub description: &'static str,
}

/// Default keybindings for Yantrik OS.
pub const DEFAULT_KEYBINDS: &[Keybind] = &[
    Keybind {
        key: "Super_L",
        action: "open-lens",
        description: "Open Intent Lens",
    },
    Keybind {
        key: "W-space",
        action: "open-lens",
        description: "Open Intent Lens (alt)",
    },
    Keybind {
        key: "W-l",
        action: "lock-screen",
        description: "Lock screen",
    },
    Keybind {
        key: "W-t",
        action: "open-terminal",
        description: "Open terminal",
    },
    Keybind {
        key: "W-e",
        action: "open-files",
        description: "Open file manager",
    },
    Keybind {
        key: "W-s",
        action: "open-settings",
        description: "Open settings",
    },
    Keybind {
        key: "Print",
        action: "screenshot",
        description: "Take screenshot (full screen)",
    },
    Keybind {
        key: "S-Print",
        action: "screenshot-region",
        description: "Take screenshot (select region)",
    },
    Keybind {
        key: "C-Print",
        action: "screenshot-clipboard",
        description: "Screenshot to clipboard (full screen)",
    },
    Keybind {
        key: "C-S-Print",
        action: "screenshot-clipboard-region",
        description: "Screenshot to clipboard (select region)",
    },
    Keybind {
        key: "W-S-q",
        action: "power-menu",
        description: "Power menu",
    },
    Keybind {
        key: "W-v",
        action: "clipboard-history",
        description: "Open clipboard history",
    },
    Keybind {
        key: "W-d",
        action: "toggle-dnd",
        description: "Toggle Do Not Disturb",
    },
    Keybind {
        key: "W-a",
        action: "app-grid",
        description: "App grid",
    },
    Keybind {
        key: "W-Tab",
        action: "window-switcher",
        description: "Window switcher",
    },
    Keybind {
        key: "A-Tab",
        action: "next-window",
        description: "Switch window",
    },
    Keybind {
        key: "A-F4",
        action: "close-window",
        description: "Close window",
    },
    // ── Snap zones (labwc-native, no D-Bus) ──
    Keybind {
        key: "W-Left",
        action: "snap-left",
        description: "Snap window to left half",
    },
    Keybind {
        key: "W-Right",
        action: "snap-right",
        description: "Snap window to right half",
    },
    Keybind {
        key: "W-Up",
        action: "snap-maximize",
        description: "Maximize window",
    },
    Keybind {
        key: "W-Down",
        action: "snap-restore",
        description: "Unsnap / restore window",
    },
];

// ── D-Bus service ──

/// D-Bus interface for receiving keybind triggers from labwc.
struct KeybindService {
    tx: Sender<SystemEvent>,
}

#[zbus::interface(name = "org.yantrik.Keybinds")]
impl KeybindService {
    /// Called by labwc via dbus-send when a keybind fires.
    fn trigger(&self, action: &str) {
        tracing::debug!(action, "Keybind triggered via D-Bus");
        let _ = self.tx.send(SystemEvent::KeybindTriggered {
            action: action.to_string(),
        });
    }
}

/// Start the keybind D-Bus service on the session bus.
/// Blocks forever (run on a dedicated thread).
pub fn run_keybind_daemon(tx: Sender<SystemEvent>) {
    if let Err(e) = start_daemon(tx) {
        tracing::error!(error = %e, "Keybind D-Bus daemon failed");
    }
}

fn start_daemon(tx: Sender<SystemEvent>) -> Result<(), zbus::Error> {
    let service = KeybindService { tx };

    let _connection = zbus::blocking::connection::Builder::session()?
        .name("org.yantrik.Keybinds")?
        .serve_at("/org/yantrik/Keybinds", service)?
        .build()?;

    tracing::info!("Keybind D-Bus service running (org.yantrik.Keybinds)");

    // Keep the thread alive — the connection stays open while _connection lives.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

// ── labwc rc.xml generator ──

/// Generate the `<keyboard>` section for labwc rc.xml.
/// Each keybind calls dbus-send to trigger the action.
pub fn generate_labwc_keybinds() -> String {
    let mut xml = String::from("  <keyboard>\n");

    for bind in DEFAULT_KEYBINDS {
        // Special cases: native labwc actions (no D-Bus roundtrip)
        match bind.action {
            "next-window" => {
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"NextWindow\" />\n    </keybind>\n",
                    bind.description, bind.key
                ));
            }
            "close-window" => {
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"Close\" />\n    </keybind>\n",
                    bind.description, bind.key
                ));
            }
            "snap-left" => {
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"SnapToEdge\"><direction>left</direction></action>\n    </keybind>\n",
                    bind.description, bind.key
                ));
            }
            "snap-right" => {
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"SnapToEdge\"><direction>right</direction></action>\n    </keybind>\n",
                    bind.description, bind.key
                ));
            }
            "snap-maximize" => {
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"Maximize\" />\n    </keybind>\n",
                    bind.description, bind.key
                ));
            }
            "snap-restore" => {
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"UnMaximize\" />\n    </keybind>\n",
                    bind.description, bind.key
                ));
            }
            _ => {
                // Route through D-Bus to yantrik
                xml.push_str(&format!(
                    "    <!-- {} -->\n    <keybind key=\"{}\">\n      <action name=\"Execute\">\n        <command>dbus-send --session --type=method_call --dest=org.yantrik.Keybinds /org/yantrik/Keybinds org.yantrik.Keybinds.Trigger string:{}</command>\n      </action>\n    </keybind>\n",
                    bind.description, bind.key, bind.action
                ));
            }
        }
    }

    xml.push_str("  </keyboard>");
    xml
}

/// Write the complete labwc rc.xml file if it doesn't already exist.
/// Called at startup to ensure keybindings are configured.
pub fn ensure_labwc_config() {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };

    let config_dir = std::path::PathBuf::from(&home).join(".config/labwc");
    let rc_path = config_dir.join("rc.xml");

    // Only write if the file doesn't exist (don't overwrite user customizations)
    if rc_path.exists() {
        tracing::debug!("labwc rc.xml already exists, skipping keybind generation");
        return;
    }

    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        tracing::warn!(error = %e, "Could not create labwc config dir");
        return;
    }

    let keyboard_section = generate_labwc_keybinds();

    let rc_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<labwc_config>
  <core>
    <gap>0</gap>
  </core>

  <theme>
    <name></name>
    <titlebar>
      <font name="DejaVu Sans" size="10" />
    </titlebar>
  </theme>

{keyboard_section}

  <windowRules>
    <!-- Yantrik shell starts maximized (acts as desktop) -->
    <windowRule identifier="yantrik-ui">
      <action name="Maximize" />
    </windowRule>
  </windowRules>
</labwc_config>
"#
    );

    match std::fs::write(&rc_path, rc_xml) {
        Ok(_) => tracing::info!(path = %rc_path.display(), "Generated labwc rc.xml with keybindings"),
        Err(e) => tracing::warn!(error = %e, "Failed to write labwc rc.xml"),
    }
}
