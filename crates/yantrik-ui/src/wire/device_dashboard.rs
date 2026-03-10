//! Device Dashboard wire module — USB, PCI, Input, Storage, Display hardware overview.
//!
//! Parses lsusb, lspci, /proc/bus/input/devices, lsblk, and /sys/class/drm/
//! for comprehensive hardware information. Background thread scanning with
//! 10-second refresh timer. Screen 27.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, DisplayDeviceData, InputDeviceData, PciDeviceData, StorageDeviceData, UsbDeviceData};

// ═══════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct UsbDevice {
    bus: String,
    device_id: String,
    vendor_product: String,
    description: String,
}

#[derive(Clone)]
struct PciDevice {
    slot: String,
    class_name: String,
    vendor: String,
    device_name: String,
    class_color: slint::Color,
}

#[derive(Clone)]
struct InputDevice {
    name: String,
    handler: String,
    device_type: String,
    phys: String,
}

#[derive(Clone)]
struct StorageDevice {
    name: String,
    size_text: String,
    device_type: String,
    mountpoint: String,
    filesystem: String,
    usage_percent: f32,
    smart_status: String,
}

#[derive(Clone)]
struct DisplayDevice {
    name: String,
    resolution: String,
    connector: String,
    status: String,
}

struct DeviceState {
    usb_devices: Vec<UsbDevice>,
    pci_devices: Vec<PciDevice>,
    input_devices: Vec<InputDevice>,
    storage_devices: Vec<StorageDevice>,
    display_devices: Vec<DisplayDevice>,
    gpu_info: String,
    dirty: bool,
    scanning: bool,
    search_filter: String,
    /// Previous total device count for hotplug detection.
    prev_device_count: usize,
    /// Hotplug notification text.
    hotplug_notification: String,
    /// Whether hotplug banner should be visible.
    hotplug_banner_visible: bool,
}

impl DeviceState {
    fn new() -> Self {
        Self {
            usb_devices: Vec::new(),
            pci_devices: Vec::new(),
            input_devices: Vec::new(),
            storage_devices: Vec::new(),
            display_devices: Vec::new(),
            gpu_info: String::new(),
            dirty: true,
            scanning: true,
            search_filter: String::new(),
            prev_device_count: 0,
            hotplug_notification: String::new(),
            hotplug_banner_visible: false,
        }
    }

    fn total_device_count(&self) -> usize {
        self.usb_devices.len()
            + self.pci_devices.len()
            + self.input_devices.len()
            + self.storage_devices.len()
            + self.display_devices.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Command helpers
// ═══════════════════════════════════════════════════════════════════════

fn cmd_output(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

// ═══════════════════════════════════════════════════════════════════════
// Search filter helper
// ═══════════════════════════════════════════════════════════════════════

fn matches_filter(filter: &str, fields: &[&str]) -> bool {
    if filter.is_empty() {
        return true;
    }
    let filter_lower = filter.to_lowercase();
    fields.iter().any(|f| f.to_lowercase().contains(&filter_lower))
}

// ═══════════════════════════════════════════════════════════════════════
// Parsers
// ═══════════════════════════════════════════════════════════════════════

/// Parse `lsusb` output.
/// Format: Bus 001 Device 002: ID 1d6b:0002 Linux Foundation 2.0 root hub
fn parse_lsusb() -> Vec<UsbDevice> {
    let output = cmd_output("lsusb", &[]);
    if output.is_empty() {
        return Vec::new();
    }

    let mut devices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse "Bus XXX Device YYY: ID VVVV:PPPP Description..."
        let bus = line
            .get(4..7)
            .unwrap_or("")
            .trim()
            .to_string();

        let device_id = line
            .find("Device ")
            .and_then(|i| line.get(i + 7..i + 10))
            .unwrap_or("")
            .trim()
            .trim_end_matches(':')
            .to_string();

        let (vendor_product, description) = if let Some(id_pos) = line.find("ID ") {
            let after_id = &line[id_pos + 3..];
            if let Some(space_pos) = after_id.find(' ') {
                let vp = after_id[..space_pos].trim().to_string();
                let desc = after_id[space_pos..].trim().to_string();
                (vp, desc)
            } else {
                (after_id.trim().to_string(), String::new())
            }
        } else {
            (String::new(), line.to_string())
        };

        devices.push(UsbDevice {
            bus: format!("Bus {}", bus),
            device_id,
            vendor_product,
            description,
        });
    }

    devices
}

/// Parse `lspci` output.
/// Format: 00:00.0 Host bridge: Intel Corporation ...
fn parse_lspci() -> Vec<PciDevice> {
    let output = cmd_output("lspci", &[]);
    if output.is_empty() {
        return Vec::new();
    }

    let mut devices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Split at first space to get slot
        let (slot, rest) = match line.split_once(' ') {
            Some(pair) => pair,
            None => continue,
        };

        // Split at first colon to get class and device info
        let (class_name, device_info) = match rest.split_once(": ") {
            Some(pair) => pair,
            None => (rest, ""),
        };

        // Try to separate vendor from device name
        let (vendor, device_name) = if let Some(pos) = device_info.find(" Corporation ") {
            let end = pos + " Corporation".len();
            (
                device_info[..end].trim().to_string(),
                device_info[end..].trim().to_string(),
            )
        } else if let Some(pos) = device_info.find(" Inc. ") {
            let end = pos + " Inc.".len();
            (
                device_info[..end].trim().to_string(),
                device_info[end..].trim().to_string(),
            )
        } else if let Some(pos) = device_info.find(" Ltd. ") {
            let end = pos + " Ltd.".len();
            (
                device_info[..end].trim().to_string(),
                device_info[end..].trim().to_string(),
            )
        } else if let Some(pos) = device_info.find("] ") {
            // Handle "[XXXX] Device name" format
            (
                device_info[..pos + 1].trim().to_string(),
                device_info[pos + 2..].trim().to_string(),
            )
        } else {
            (String::new(), device_info.trim().to_string())
        };

        let class_lower = class_name.to_lowercase();
        let class_color = pci_class_color(&class_lower);

        devices.push(PciDevice {
            slot: slot.to_string(),
            class_name: class_name.to_string(),
            vendor,
            device_name,
            class_color,
        });
    }

    devices
}

/// Map PCI class names to colors.
fn pci_class_color(class_lower: &str) -> slint::Color {
    if class_lower.contains("vga")
        || class_lower.contains("display")
        || class_lower.contains("3d")
    {
        // GPU = green
        slint::Color::from_argb_u8(255, 108, 212, 128)
    } else if class_lower.contains("network") || class_lower.contains("ethernet") || class_lower.contains("wifi") {
        // Network = blue
        slint::Color::from_argb_u8(255, 74, 184, 240)
    } else if class_lower.contains("storage")
        || class_lower.contains("sata")
        || class_lower.contains("ide")
        || class_lower.contains("nvme")
        || class_lower.contains("raid")
    {
        // Storage = orange
        slint::Color::from_argb_u8(255, 232, 160, 107)
    } else if class_lower.contains("usb") {
        // USB = cyan
        slint::Color::from_argb_u8(255, 90, 200, 212)
    } else if class_lower.contains("audio") || class_lower.contains("multimedia") {
        // Audio = purple
        slint::Color::from_argb_u8(255, 196, 139, 212)
    } else if class_lower.contains("bridge") || class_lower.contains("isa") || class_lower.contains("host") {
        // Bridge/Host = dim
        slint::Color::from_argb_u8(255, 122, 132, 148)
    } else {
        // Default = secondary
        slint::Color::from_argb_u8(255, 160, 170, 184)
    }
}

/// Parse /proc/bus/input/devices for input devices.
fn parse_input_devices() -> Vec<InputDevice> {
    let content = match std::fs::read_to_string("/proc/bus/input/devices") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut devices = Vec::new();
    let mut name = String::new();
    let mut phys = String::new();
    let mut handlers = String::new();

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() {
            // End of device block
            if !name.is_empty() {
                let device_type = classify_input_device(&name, &handlers);
                devices.push(InputDevice {
                    name: name.clone(),
                    handler: handlers.clone(),
                    device_type,
                    phys: phys.clone(),
                });
            }
            name.clear();
            phys.clear();
            handlers.clear();
            continue;
        }

        if let Some(rest) = line.strip_prefix("N: Name=") {
            name = rest.trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("P: Phys=") {
            phys = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
            handlers = rest.to_string();
        }
    }

    // Handle last device block
    if !name.is_empty() {
        let device_type = classify_input_device(&name, &handlers);
        devices.push(InputDevice {
            name: name.clone(),
            handler: handlers.clone(),
            device_type,
            phys: phys.clone(),
        });
    }

    devices
}

/// Classify an input device based on name and handlers.
fn classify_input_device(name: &str, handlers: &str) -> String {
    let name_lower = name.to_lowercase();
    let handler_lower = handlers.to_lowercase();

    if name_lower.contains("keyboard") || name_lower.contains("kbd") || handler_lower.contains("kbd") {
        "keyboard".to_string()
    } else if name_lower.contains("touchpad") || name_lower.contains("trackpad") {
        "touchpad".to_string()
    } else if name_lower.contains("mouse") || name_lower.contains("trackball") || handler_lower.contains("mouse") {
        "mouse".to_string()
    } else {
        "other".to_string()
    }
}

/// Parse `lsblk --json` for storage devices.
fn parse_lsblk() -> Vec<StorageDevice> {
    let output = cmd_output("lsblk", &["--json", "-o", "NAME,SIZE,TYPE,MOUNTPOINT,FSTYPE"]);
    if output.is_empty() {
        return parse_lsblk_fallback();
    }

    // Simple JSON parsing without serde — look for blockdevices array
    parse_lsblk_json(&output)
}

/// Parse lsblk JSON output manually.
fn parse_lsblk_json(json: &str) -> Vec<StorageDevice> {
    let mut devices = Vec::new();
    // Extract each device entry by finding name/size/type/mountpoint patterns
    // We do a simple line-by-line parse of the JSON

    let mut current_name = String::new();
    let mut current_size = String::new();
    let mut current_type = String::new();
    let mut current_mount = String::new();
    let mut current_fs = String::new();

    for line in json.lines() {
        let line = line.trim();

        if let Some(val) = extract_json_string(line, "\"name\"") {
            // If we already have a device in progress, push it
            if !current_name.is_empty() {
                let usage = compute_disk_usage(&current_mount);
                let smart = query_smart_status(&current_name, &current_type);
                devices.push(StorageDevice {
                    name: current_name.clone(),
                    size_text: current_size.clone(),
                    device_type: current_type.clone(),
                    mountpoint: current_mount.clone(),
                    filesystem: current_fs.clone(),
                    usage_percent: usage,
                    smart_status: smart,
                });
            }
            current_name = val;
            current_size.clear();
            current_type.clear();
            current_mount.clear();
            current_fs.clear();
        }
        if let Some(val) = extract_json_string(line, "\"size\"") {
            current_size = val;
        }
        if let Some(val) = extract_json_string(line, "\"type\"") {
            current_type = val;
        }
        if let Some(val) = extract_json_string(line, "\"mountpoint\"") {
            current_mount = val;
        }
        // mountpoint can also be null
        if line.contains("\"mountpoint\"") && line.contains("null") {
            current_mount.clear();
        }
        if let Some(val) = extract_json_string(line, "\"fstype\"") {
            current_fs = val;
        }
        if line.contains("\"fstype\"") && line.contains("null") {
            current_fs.clear();
        }
    }

    // Push last device
    if !current_name.is_empty() {
        let usage = compute_disk_usage(&current_mount);
        let smart = query_smart_status(&current_name, &current_type);
        devices.push(StorageDevice {
            name: current_name,
            size_text: current_size,
            device_type: current_type,
            mountpoint: current_mount,
            filesystem: current_fs,
            usage_percent: usage,
            smart_status: smart,
        });
    }

    devices
}

/// Extract a string value from a JSON line like `"key": "value"`.
fn extract_json_string(line: &str, key: &str) -> Option<String> {
    let pos = line.find(key)?;
    let after_key = &line[pos + key.len()..];
    // Skip `: "`
    let colon_pos = after_key.find(':')?;
    let after_colon = after_key[colon_pos + 1..].trim();
    if after_colon.starts_with('"') {
        let inner = &after_colon[1..];
        let end = inner.find('"')?;
        Some(inner[..end].to_string())
    } else {
        None
    }
}

/// Query SMART health status for a disk device using smartctl.
/// Returns "Healthy", "Warning", "Critical", or "" if not applicable/available.
fn query_smart_status(name: &str, device_type: &str) -> String {
    // Only check whole-disk devices, not partitions or LVM
    if device_type != "disk" {
        return String::new();
    }

    let dev_path = format!("/dev/{}", name);
    let output = cmd_output("smartctl", &["-H", &dev_path]);
    if output.is_empty() {
        return String::new();
    }

    let output_lower = output.to_lowercase();

    // smartctl returns "SMART overall-health self-assessment test result: PASSED"
    if output_lower.contains("passed") || output_lower.contains("ok") {
        // Check for pre-fail attributes indicating warnings
        let attr_output = cmd_output("smartctl", &["-A", &dev_path]);
        if !attr_output.is_empty() {
            // Look for reallocated sectors, pending sectors, or uncorrectable errors
            let has_warnings = attr_output.lines().any(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 10 {
                    // Check attribute IDs 5 (Reallocated), 197 (Pending), 198 (Uncorrectable)
                    let id = parts.first().unwrap_or(&"");
                    let raw_value = parts.last().unwrap_or(&"0");
                    if (*id == "5" || *id == "197" || *id == "198") {
                        if let Ok(val) = raw_value.parse::<u64>() {
                            return val > 0;
                        }
                    }
                }
                false
            });
            if has_warnings {
                return "Warning".to_string();
            }
        }
        "Healthy".to_string()
    } else if output_lower.contains("failed") {
        "Critical".to_string()
    } else {
        String::new()
    }
}

/// Compute disk usage percentage for a mountpoint using statvfs.
fn compute_disk_usage(mountpoint: &str) -> f32 {
    if mountpoint.is_empty() {
        return 0.0;
    }
    match statvfs_usage(mountpoint) {
        Some((used, total)) if total > 0 => (used as f64 / total as f64 * 100.0) as f32,
        _ => 0.0,
    }
}

/// Call statvfs and return (used_bytes, total_bytes).
fn statvfs_usage(path: &str) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let c_path = CString::new(path).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let block_size = stat.f_frsize as u64;
            let total = stat.f_blocks as u64 * block_size;
            let avail = stat.f_bavail as u64 * block_size;
            let used = total.saturating_sub(avail);
            Some((used, total))
        } else {
            None
        }
    }
}

/// Fallback: parse plain `lsblk` output if --json is not available.
fn parse_lsblk_fallback() -> Vec<StorageDevice> {
    let output = cmd_output("lsblk", &["-o", "NAME,SIZE,TYPE,MOUNTPOINT,FSTYPE", "--noheadings"]);
    if output.is_empty() {
        return Vec::new();
    }

    let mut devices = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let name = parts[0].trim_start_matches(['-', '`', '|', ' ']).to_string();
        let size = parts.get(1).unwrap_or(&"").to_string();
        let dtype = parts.get(2).unwrap_or(&"").to_string();
        let mount = parts.get(3).unwrap_or(&"").to_string();
        let fs = parts.get(4).unwrap_or(&"").to_string();

        let usage = compute_disk_usage(&mount);
        let smart = query_smart_status(&name, &dtype);

        devices.push(StorageDevice {
            name,
            size_text: size,
            device_type: dtype,
            mountpoint: mount,
            filesystem: fs,
            usage_percent: usage,
            smart_status: smart,
        });
    }

    devices
}

/// Read display outputs from /sys/class/drm/.
fn parse_display_devices() -> (Vec<DisplayDevice>, String) {
    let mut devices = Vec::new();
    let mut gpu_info = String::new();

    // GPU info from lspci
    let lspci_output = cmd_output("lspci", &[]);
    for line in lspci_output.lines() {
        let lower = line.to_lowercase();
        if lower.contains("vga") || lower.contains("3d controller") || lower.contains("display") {
            // Strip the slot prefix for cleaner display
            if let Some((_slot, rest)) = line.split_once(' ') {
                if let Some((_class, info)) = rest.split_once(": ") {
                    if gpu_info.is_empty() {
                        gpu_info = info.trim().to_string();
                    } else {
                        gpu_info.push_str(" | ");
                        gpu_info.push_str(info.trim());
                    }
                }
            }
        }
    }

    // Read /sys/class/drm/*/status and modes
    let drm_path = std::path::Path::new("/sys/class/drm");
    if let Ok(entries) = std::fs::read_dir(drm_path) {
        for entry in entries.flatten() {
            let entry_name = entry.file_name().to_string_lossy().to_string();

            // Skip entries that are just "card0" without a connector suffix
            if !entry_name.contains('-') {
                continue;
            }

            // Parse connector from name like "card0-HDMI-A-1", "card0-DP-1", "card0-eDP-1"
            let connector = if let Some(pos) = entry_name.find('-') {
                entry_name[pos + 1..].to_string()
            } else {
                entry_name.clone()
            };

            let dir = entry.path();

            // Read status
            let status = std::fs::read_to_string(dir.join("status"))
                .unwrap_or_default()
                .trim()
                .to_string();

            // Read modes (first line = preferred mode)
            let resolution = std::fs::read_to_string(dir.join("modes"))
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();

            // Read enabled state
            let enabled = std::fs::read_to_string(dir.join("enabled"))
                .unwrap_or_default()
                .trim()
                .to_string();

            let display_name = if !enabled.is_empty() && enabled != "disabled" {
                format!("{} ({})", connector, enabled)
            } else {
                connector.clone()
            };

            devices.push(DisplayDevice {
                name: display_name,
                resolution,
                connector,
                status,
            });
        }
    }

    // Sort: connected first
    devices.sort_by(|a, b| {
        let a_conn = a.status == "connected";
        let b_conn = b.status == "connected";
        b_conn.cmp(&a_conn)
    });

    (devices, gpu_info)
}

/// Refresh all device data.
fn refresh_all(state: &Arc<Mutex<DeviceState>>) {
    if let Ok(mut s) = state.lock() {
        s.scanning = true;
        s.dirty = true;
    }

    let usb = parse_lsusb();
    let pci = parse_lspci();
    let input = parse_input_devices();
    let storage = parse_lsblk();
    let (display, gpu) = parse_display_devices();

    if let Ok(mut s) = state.lock() {
        s.usb_devices = usb;
        s.pci_devices = pci;
        s.input_devices = input;
        s.storage_devices = storage;
        s.display_devices = display;
        s.gpu_info = gpu;
        s.scanning = false;
        s.dirty = true;

        // Hotplug detection: compare total device count
        let new_count = s.total_device_count();
        if s.prev_device_count > 0 && new_count != s.prev_device_count {
            if new_count > s.prev_device_count {
                let diff = new_count - s.prev_device_count;
                s.hotplug_notification = format!(
                    "New device detected ({} device{} added)",
                    diff,
                    if diff == 1 { "" } else { "s" }
                );
            } else {
                let diff = s.prev_device_count - new_count;
                s.hotplug_notification = format!(
                    "Device removed ({} device{} disconnected)",
                    diff,
                    if diff == 1 { "" } else { "s" }
                );
            }
            s.hotplug_banner_visible = true;
        }
        s.prev_device_count = new_count;
    }
}

/// Export all device data to JSON file at ~/devices.json.
fn export_devices(state: &Arc<Mutex<DeviceState>>) -> Result<String, String> {
    let s = state.lock().map_err(|e| format!("lock error: {}", e))?;

    let mut json = String::from("{\n");

    // USB devices
    json.push_str("  \"usb_devices\": [\n");
    for (i, d) in s.usb_devices.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"bus\": \"{}\", \"device_id\": \"{}\", \"vendor_product\": \"{}\", \"description\": \"{}\"}}{}",
            escape_json(&d.bus), escape_json(&d.device_id),
            escape_json(&d.vendor_product), escape_json(&d.description),
            if i + 1 < s.usb_devices.len() { ",\n" } else { "\n" }
        ));
    }
    json.push_str("  ],\n");

    // PCI devices
    json.push_str("  \"pci_devices\": [\n");
    for (i, d) in s.pci_devices.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"slot\": \"{}\", \"class\": \"{}\", \"vendor\": \"{}\", \"device\": \"{}\"}}{}",
            escape_json(&d.slot), escape_json(&d.class_name),
            escape_json(&d.vendor), escape_json(&d.device_name),
            if i + 1 < s.pci_devices.len() { ",\n" } else { "\n" }
        ));
    }
    json.push_str("  ],\n");

    // Input devices
    json.push_str("  \"input_devices\": [\n");
    for (i, d) in s.input_devices.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"name\": \"{}\", \"handler\": \"{}\", \"type\": \"{}\", \"phys\": \"{}\"}}{}",
            escape_json(&d.name), escape_json(&d.handler),
            escape_json(&d.device_type), escape_json(&d.phys),
            if i + 1 < s.input_devices.len() { ",\n" } else { "\n" }
        ));
    }
    json.push_str("  ],\n");

    // Storage devices
    json.push_str("  \"storage_devices\": [\n");
    for (i, d) in s.storage_devices.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"name\": \"{}\", \"size\": \"{}\", \"type\": \"{}\", \"mountpoint\": \"{}\", \"filesystem\": \"{}\", \"usage_percent\": {:.1}, \"smart_status\": \"{}\"}}{}",
            escape_json(&d.name), escape_json(&d.size_text),
            escape_json(&d.device_type), escape_json(&d.mountpoint),
            escape_json(&d.filesystem), d.usage_percent,
            escape_json(&d.smart_status),
            if i + 1 < s.storage_devices.len() { ",\n" } else { "\n" }
        ));
    }
    json.push_str("  ],\n");

    // Display devices
    json.push_str("  \"display_devices\": [\n");
    for (i, d) in s.display_devices.iter().enumerate() {
        json.push_str(&format!(
            "    {{\"name\": \"{}\", \"resolution\": \"{}\", \"connector\": \"{}\", \"status\": \"{}\"}}{}",
            escape_json(&d.name), escape_json(&d.resolution),
            escape_json(&d.connector), escape_json(&d.status),
            if i + 1 < s.display_devices.len() { ",\n" } else { "\n" }
        ));
    }
    json.push_str("  ],\n");

    // GPU info
    json.push_str(&format!("  \"gpu_info\": \"{}\"\n", escape_json(&s.gpu_info)));
    json.push_str("}\n");

    // Write to ~/devices.json
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let path = format!("{}/devices.json", home);
    std::fs::write(&path, &json).map_err(|e| format!("write error: {}", e))?;

    Ok(format!("Exported to {}", path))
}

/// Escape a string for JSON output.
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Generate detail info text for a USB device.
fn generate_usb_detail(dev: &UsbDevice) -> String {
    let bus_num = dev.bus.trim_start_matches("Bus ").trim();
    let dev_num = dev.device_id.trim();
    let verbose = cmd_output(
        "lsusb",
        &["-v", "-s", &format!("{}:{}", bus_num, dev_num)],
    );
    if verbose.is_empty() {
        format!(
            "Name: {}\nBus: {}\nDevice: {}\nID: {}\nStatus: Connected",
            dev.description, dev.bus, dev.device_id, dev.vendor_product
        )
    } else {
        // Extract key fields from verbose output
        let mut detail = format!("Name: {}\n", dev.description);
        detail.push_str(&format!("Bus Path: {}:{}\n", bus_num, dev_num));
        detail.push_str(&format!("Vendor:Product: {}\n", dev.vendor_product));

        // Extract driver/module info
        for line in verbose.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("bcdUSB") || trimmed.starts_with("bDeviceClass")
                || trimmed.starts_with("bDeviceSubClass") || trimmed.starts_with("bDeviceProtocol")
                || trimmed.starts_with("idVendor") || trimmed.starts_with("idProduct")
                || trimmed.starts_with("iManufacturer") || trimmed.starts_with("iProduct")
                || trimmed.starts_with("iSerial") || trimmed.starts_with("bMaxPower")
            {
                detail.push_str(&format!("{}\n", trimmed));
            }
        }

        detail.push_str("\nStatus: Connected");
        detail
    }
}

/// Generate detail info text for a PCI device.
fn generate_pci_detail(dev: &PciDevice) -> String {
    // Get verbose PCI info
    let verbose = cmd_output("lspci", &["-v", "-s", &dev.slot]);

    let mut detail = format!("Name: {}\n", dev.device_name);
    detail.push_str(&format!("Vendor: {}\n", dev.vendor));
    detail.push_str(&format!("Class: {}\n", dev.class_name));
    detail.push_str(&format!("Bus Path: {}\n", dev.slot));

    if !verbose.is_empty() {
        // Extract driver, memory regions, IRQ
        for line in verbose.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Kernel driver") || trimmed.starts_with("Kernel modules")
                || trimmed.starts_with("Subsystem") || trimmed.starts_with("Flags")
                || trimmed.starts_with("Memory") || trimmed.starts_with("I/O ports")
                || trimmed.starts_with("IRQ") || trimmed.starts_with("Capabilities")
            {
                detail.push_str(&format!("{}\n", trimmed));
            }
        }
    }

    detail.push_str("\nStatus: Active");
    detail
}

// ═══════════════════════════════════════════════════════════════════════
// UI sync
// ═══════════════════════════════════════════════════════════════════════

fn sync_to_ui(ui: &App, state: &Arc<Mutex<DeviceState>>) {
    let s = match state.lock() {
        Ok(mut s) => {
            if !s.dirty {
                return;
            }
            s.dirty = false;
            // Clone data for UI
            DeviceSnapshot {
                usb_devices: s.usb_devices.clone(),
                pci_devices: s.pci_devices.clone(),
                input_devices: s.input_devices.clone(),
                storage_devices: s.storage_devices.clone(),
                display_devices: s.display_devices.clone(),
                gpu_info: s.gpu_info.clone(),
                scanning: s.scanning,
                search_filter: s.search_filter.clone(),
                hotplug_notification: s.hotplug_notification.clone(),
                hotplug_banner_visible: s.hotplug_banner_visible,
            }
        }
        Err(_) => return,
    };

    let filter = &s.search_filter;

    // USB — apply search filter
    let usb_data: Vec<UsbDeviceData> = s
        .usb_devices
        .iter()
        .enumerate()
        .filter(|(_, d)| matches_filter(filter, &[&d.bus, &d.vendor_product, &d.description]))
        .map(|(i, d)| {
            let selected_idx = ui.get_dev_usb_selected_index();
            UsbDeviceData {
                bus: d.bus.clone().into(),
                device_id: d.device_id.clone().into(),
                vendor_product: d.vendor_product.clone().into(),
                description: d.description.clone().into(),
                is_selected: i as i32 == selected_idx,
            }
        })
        .collect();
    ui.set_dev_usb_devices(ModelRc::new(VecModel::from(usb_data)));

    // PCI — apply search filter
    let pci_data: Vec<PciDeviceData> = s
        .pci_devices
        .iter()
        .filter(|d| matches_filter(filter, &[&d.slot, &d.class_name, &d.vendor, &d.device_name]))
        .map(|d| PciDeviceData {
            slot: d.slot.clone().into(),
            class_name: d.class_name.clone().into(),
            vendor: d.vendor.clone().into(),
            device_name: d.device_name.clone().into(),
            class_color: d.class_color,
        })
        .collect();
    ui.set_dev_pci_devices(ModelRc::new(VecModel::from(pci_data)));

    // Input — apply search filter
    let input_data: Vec<InputDeviceData> = s
        .input_devices
        .iter()
        .filter(|d| matches_filter(filter, &[&d.name, &d.handler, &d.device_type, &d.phys]))
        .map(|d| InputDeviceData {
            name: d.name.clone().into(),
            handler: d.handler.clone().into(),
            device_type: d.device_type.clone().into(),
            phys: d.phys.clone().into(),
        })
        .collect();
    ui.set_dev_input_devices(ModelRc::new(VecModel::from(input_data)));

    // Storage — apply search filter
    let storage_data: Vec<StorageDeviceData> = s
        .storage_devices
        .iter()
        .filter(|d| matches_filter(filter, &[&d.name, &d.device_type, &d.mountpoint, &d.filesystem]))
        .map(|d| StorageDeviceData {
            name: d.name.clone().into(),
            size_text: d.size_text.clone().into(),
            device_type: d.device_type.clone().into(),
            mountpoint: d.mountpoint.clone().into(),
            filesystem: d.filesystem.clone().into(),
            usage_percent: d.usage_percent,
            smart_status: d.smart_status.clone().into(),
        })
        .collect();
    ui.set_dev_storage_devices(ModelRc::new(VecModel::from(storage_data)));

    // Display — apply search filter
    let display_data: Vec<DisplayDeviceData> = s
        .display_devices
        .iter()
        .filter(|d| matches_filter(filter, &[&d.name, &d.connector, &d.resolution, &d.status]))
        .map(|d| DisplayDeviceData {
            name: d.name.clone().into(),
            resolution: d.resolution.clone().into(),
            connector: d.connector.clone().into(),
            status: d.status.clone().into(),
        })
        .collect();
    ui.set_dev_display_devices(ModelRc::new(VecModel::from(display_data)));

    // GPU info
    ui.set_dev_gpu_info(s.gpu_info.into());

    // Scanning state
    ui.set_dev_scanning(s.scanning);

    // Summary text
    let total = ui.get_dev_usb_devices().row_count()
        + ui.get_dev_pci_devices().row_count()
        + ui.get_dev_input_devices().row_count()
        + ui.get_dev_storage_devices().row_count()
        + ui.get_dev_display_devices().row_count();
    ui.set_dev_summary(format!("{} total devices", total).into());

    // Hotplug notification
    ui.set_dev_hotplug_notification(s.hotplug_notification.clone().into());
    ui.set_dev_hotplug_count(s.usb_devices.len() as i32);
    if s.hotplug_banner_visible {
        ui.set_dev_hotplug_banner_visible(true);
    }
}

struct DeviceSnapshot {
    usb_devices: Vec<UsbDevice>,
    pci_devices: Vec<PciDevice>,
    input_devices: Vec<InputDevice>,
    storage_devices: Vec<StorageDevice>,
    display_devices: Vec<DisplayDevice>,
    gpu_info: String,
    scanning: bool,
    search_filter: String,
    hotplug_notification: String,
    hotplug_banner_visible: bool,
}

// ═══════════════════════════════════════════════════════════════════════
// Wire
// ═══════════════════════════════════════════════════════════════════════

/// Wire all Device Dashboard callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let state = Arc::new(Mutex::new(DeviceState::new()));

    // Initial refresh in background
    {
        let state_clone = state.clone();
        std::thread::spawn(move || {
            refresh_all(&state_clone);
        });
    }

    // 10-second refresh timer
    let refresh_timer = Timer::default();
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        refresh_timer.start(TimerMode::Repeated, Duration::from_secs(10), move || {
            // Sync to UI
            if let Some(ui) = ui_weak.upgrade() {
                // Only update when Device Dashboard (screen 27) is active
                if ui.get_current_screen() != 27 {
                    return;
                }
                sync_to_ui(&ui, &state_clone);
            }

            // Trigger background refresh
            let state_bg = state_clone.clone();
            std::thread::spawn(move || {
                refresh_all(&state_bg);
            });
        });
    }
    std::mem::forget(refresh_timer);

    // Initial sync after short delay
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        let init_timer = Timer::default();
        init_timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &state_clone);
            }
        });
        std::mem::forget(init_timer);
    }

    // ── Tab switch callback ──
    {
        let ui_weak = ui.as_weak();
        ui.on_dev_tab(move |tab| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dev_active_tab(tab);
            }
        });
    }

    // ── USB device select callback ──
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dev_select_usb(move |idx| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dev_usb_selected_index(idx);
                ui.set_dev_pci_selected_index(-1);

                // Generate detail text for selected USB device
                let detail = if let Ok(s) = state_clone.lock() {
                    if let Some(dev) = s.usb_devices.get(idx as usize) {
                        generate_usb_detail(dev)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                ui.set_dev_usb_detail_text(detail.clone().into());
                ui.set_dev_selected_device_info(detail.into());

                // Update selected state in USB list
                if let Ok(s) = state_clone.lock() {
                    let filter = &s.search_filter;
                    let usb_data: Vec<UsbDeviceData> = s
                        .usb_devices
                        .iter()
                        .enumerate()
                        .filter(|(_, d)| matches_filter(filter, &[&d.bus, &d.vendor_product, &d.description]))
                        .map(|(i, d)| UsbDeviceData {
                            bus: d.bus.clone().into(),
                            device_id: d.device_id.clone().into(),
                            vendor_product: d.vendor_product.clone().into(),
                            description: d.description.clone().into(),
                            is_selected: i as i32 == idx,
                        })
                        .collect();
                    ui.set_dev_usb_devices(ModelRc::new(VecModel::from(usb_data)));
                }
            }
        });
    }

    // ── PCI device select callback ──
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dev_select_pci(move |idx| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dev_pci_selected_index(idx);
                ui.set_dev_usb_selected_index(-1);

                // Generate detail text for selected PCI device
                let detail = if let Ok(s) = state_clone.lock() {
                    if let Some(dev) = s.pci_devices.get(idx as usize) {
                        generate_pci_detail(dev)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                ui.set_dev_selected_device_info(detail.into());
            }
        });
    }

    // ── Refresh callback ──
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dev_refresh(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dev_scanning(true);
            }
            let state_bg = state_clone.clone();
            std::thread::spawn(move || {
                refresh_all(&state_bg);
            });
        });
    }

    // ── Export callback ──
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dev_export(move || {
            let result = export_devices(&state_clone);
            if let Some(ui) = ui_weak.upgrade() {
                let msg = match result {
                    Ok(path) => path,
                    Err(e) => format!("Export failed: {}", e),
                };
                ui.set_dev_export_status(msg.into());

                // Clear status after 3 seconds
                let ui_weak2 = ui.as_weak();
                let clear_timer = Timer::default();
                clear_timer.start(TimerMode::SingleShot, Duration::from_secs(3), move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        ui.set_dev_export_status("".into());
                    }
                });
                std::mem::forget(clear_timer);
            }
        });
    }

    // ── Search callback ──
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dev_search(move |text| {
            let filter_str = text.to_string();
            if let Ok(mut s) = state_clone.lock() {
                s.search_filter = filter_str;
                s.dirty = true;
            }
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &state_clone);
            }
        });
    }

    // ── Close detail panel callback ──
    {
        let ui_weak = ui.as_weak();
        ui.on_dev_close_detail(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dev_usb_selected_index(-1);
                ui.set_dev_pci_selected_index(-1);
                ui.set_dev_selected_device_info("".into());
                ui.set_dev_usb_detail_text("".into());
            }
        });
    }

    // ── Dismiss hotplug notification callback ──
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_dev_dismiss_hotplug(move || {
            if let Ok(mut s) = state_clone.lock() {
                s.hotplug_banner_visible = false;
                s.hotplug_notification.clear();
            }
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dev_hotplug_banner_visible(false);
            }
        });
    }

    // ── AI Explain callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_dev_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let gpu = ui.get_dev_gpu_info().to_string();
        let device_detail = ui.get_dev_selected_device_info().to_string();
        let usb_detail = ui.get_dev_usb_detail_text().to_string();

        let detail_text = if !device_detail.is_empty() {
            &device_detail
        } else if !usb_detail.is_empty() {
            &usb_detail
        } else {
            "no device selected"
        };

        let context = format!(
            "GPU: {}\nSelected device details:\n{}",
            if gpu.is_empty() { "none detected" } else { &gpu },
            detail_text
        );
        let prompt = super::ai_assist::device_analysis_prompt(&context);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_dev_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_dev_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_dev_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_dev_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_dev_ai_panel_open(false);
        }
    });
}
