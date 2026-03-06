//! Device Dashboard wire module — USB, PCI, Input, Storage, Display hardware overview.
//!
//! Parses lsusb, lspci, /proc/bus/input/devices, lsblk, and /sys/class/drm/
//! for comprehensive hardware information. Background thread scanning with
//! 10-second refresh timer. Screen 27.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

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
        }
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
                devices.push(StorageDevice {
                    name: current_name.clone(),
                    size_text: current_size.clone(),
                    device_type: current_type.clone(),
                    mountpoint: current_mount.clone(),
                    filesystem: current_fs.clone(),
                    usage_percent: usage,
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
        devices.push(StorageDevice {
            name: current_name,
            size_text: current_size,
            device_type: current_type,
            mountpoint: current_mount,
            filesystem: current_fs,
            usage_percent: usage,
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

        devices.push(StorageDevice {
            name,
            size_text: size,
            device_type: dtype,
            mountpoint: mount,
            filesystem: fs,
            usage_percent: usage,
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
    }
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
            }
        }
        Err(_) => return,
    };

    // USB
    let usb_data: Vec<UsbDeviceData> = s
        .usb_devices
        .iter()
        .enumerate()
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

    // PCI
    let pci_data: Vec<PciDeviceData> = s
        .pci_devices
        .iter()
        .map(|d| PciDeviceData {
            slot: d.slot.clone().into(),
            class_name: d.class_name.clone().into(),
            vendor: d.vendor.clone().into(),
            device_name: d.device_name.clone().into(),
            class_color: d.class_color,
        })
        .collect();
    ui.set_dev_pci_devices(ModelRc::new(VecModel::from(pci_data)));

    // Input
    let input_data: Vec<InputDeviceData> = s
        .input_devices
        .iter()
        .map(|d| InputDeviceData {
            name: d.name.clone().into(),
            handler: d.handler.clone().into(),
            device_type: d.device_type.clone().into(),
            phys: d.phys.clone().into(),
        })
        .collect();
    ui.set_dev_input_devices(ModelRc::new(VecModel::from(input_data)));

    // Storage
    let storage_data: Vec<StorageDeviceData> = s
        .storage_devices
        .iter()
        .map(|d| StorageDeviceData {
            name: d.name.clone().into(),
            size_text: d.size_text.clone().into(),
            device_type: d.device_type.clone().into(),
            mountpoint: d.mountpoint.clone().into(),
            filesystem: d.filesystem.clone().into(),
            usage_percent: d.usage_percent,
        })
        .collect();
    ui.set_dev_storage_devices(ModelRc::new(VecModel::from(storage_data)));

    // Display
    let display_data: Vec<DisplayDeviceData> = s
        .display_devices
        .iter()
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
}

struct DeviceSnapshot {
    usb_devices: Vec<UsbDevice>,
    pci_devices: Vec<PciDevice>,
    input_devices: Vec<InputDevice>,
    storage_devices: Vec<StorageDevice>,
    display_devices: Vec<DisplayDevice>,
    gpu_info: String,
    scanning: bool,
}

// ═══════════════════════════════════════════════════════════════════════
// Wire
// ═══════════════════════════════════════════════════════════════════════

/// Wire all Device Dashboard callbacks.
pub fn wire(ui: &App, _ctx: &AppContext) {
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

                // Generate detail text for selected USB device
                let detail = if let Ok(s) = state_clone.lock() {
                    if let Some(dev) = s.usb_devices.get(idx as usize) {
                        // Try to get verbose info via lsusb -v -s bus:device
                        let bus_num = dev.bus.trim_start_matches("Bus ").trim();
                        let dev_num = dev.device_id.trim();
                        let verbose = cmd_output(
                            "lsusb",
                            &["-v", "-s", &format!("{}:{}", bus_num, dev_num)],
                        );
                        if verbose.is_empty() {
                            format!(
                                "Bus: {}\nDevice: {}\nID: {}\nDescription: {}",
                                dev.bus, dev.device_id, dev.vendor_product, dev.description
                            )
                        } else {
                            // Truncate verbose output to keep it manageable
                            let truncated: String = verbose.lines().take(20).collect::<Vec<_>>().join("\n");
                            truncated
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                ui.set_dev_usb_detail_text(detail.into());

                // Update selected state in USB list
                if let Ok(s) = state_clone.lock() {
                    let usb_data: Vec<UsbDeviceData> = s
                        .usb_devices
                        .iter()
                        .enumerate()
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
        let ui_weak = ui.as_weak();
        ui.on_dev_select_pci(move |_idx| {
            // PCI select — could show detail card in future
            if let Some(_ui) = ui_weak.upgrade() {
                // Currently a no-op, tab already visible
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
}
