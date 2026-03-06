//! Permission Dashboard wire module — users, groups, SUID/SGID, world-writable.
//!
//! Parses /etc/passwd and /etc/group for user/group data. Runs `find` commands
//! in background threads to locate SUID/SGID and world-writable files.
//! 30-second refresh interval for filesystem scans.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, GroupData, PermFileData, UserData};

// ── Internal data structs ──

#[derive(Clone)]
struct UserInfo {
    username: String,
    uid: i32,
    gid: i32,
    home: String,
    shell: String,
    is_system: bool,
}

#[derive(Clone)]
struct GroupInfo {
    name: String,
    gid: i32,
    member_count: i32,
    members: String,
}

#[derive(Clone)]
struct PermFileInfo {
    path: String,
    owner: String,
    group_name: String,
    perms: String,
    size_text: String,
    risk: String,
}

/// Shared permission state — updated by background threads, read by UI timer.
struct PermState {
    users: Vec<UserInfo>,
    groups: Vec<GroupInfo>,
    suid_files: Vec<PermFileInfo>,
    world_writable: Vec<PermFileInfo>,
    scanning: bool,
    dirty: bool,
}

impl PermState {
    fn new() -> Self {
        Self {
            users: Vec::new(),
            groups: Vec::new(),
            suid_files: Vec::new(),
            world_writable: Vec::new(),
            scanning: false,
            dirty: true,
        }
    }
}

/// Run a command and return stdout, or empty string on failure.
fn cmd_output(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// Parse /etc/passwd for user data.
fn parse_users() -> Vec<UserInfo> {
    let content = match std::fs::read_to_string("/etc/passwd") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut users = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 7 {
            continue;
        }
        let uid = parts[2].parse::<i32>().unwrap_or(-1);
        let gid = parts[3].parse::<i32>().unwrap_or(-1);
        users.push(UserInfo {
            username: parts[0].to_string(),
            uid,
            gid,
            home: parts[5].to_string(),
            shell: parts[6].to_string(),
            is_system: uid < 1000 && uid != 0 || parts[6].contains("nologin") || parts[6].contains("false"),
        });
    }

    // Sort: root first, then regular users, then system users
    users.sort_by(|a, b| {
        let a_priority = if a.uid == 0 {
            0
        } else if !a.is_system {
            1
        } else {
            2
        };
        let b_priority = if b.uid == 0 {
            0
        } else if !b.is_system {
            1
        } else {
            2
        };
        a_priority.cmp(&b_priority).then(a.uid.cmp(&b.uid))
    });

    users
}

/// Parse /etc/group for group data.
fn parse_groups() -> Vec<GroupInfo> {
    let content = match std::fs::read_to_string("/etc/group") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut groups = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 4 {
            continue;
        }
        let gid = parts[2].parse::<i32>().unwrap_or(-1);
        let members_str = parts[3].to_string();
        let member_count = if members_str.is_empty() {
            0
        } else {
            members_str.split(',').count() as i32
        };
        groups.push(GroupInfo {
            name: parts[0].to_string(),
            gid,
            member_count,
            members: members_str,
        });
    }

    // Sort by GID
    groups.sort_by_key(|g| g.gid);
    groups
}

/// Format file size as human-readable.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Convert octal mode to rwx string representation.
fn mode_to_string(mode: u32) -> String {
    let mut s = String::with_capacity(10);

    // File type
    if mode & 0o40000 != 0 {
        s.push('d');
    } else if mode & 0o120000 == 0o120000 {
        s.push('l');
    } else {
        s.push('-');
    }

    // Owner
    s.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    if mode & 0o4000 != 0 {
        s.push(if mode & 0o100 != 0 { 's' } else { 'S' });
    } else {
        s.push(if mode & 0o100 != 0 { 'x' } else { '-' });
    }

    // Group
    s.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    if mode & 0o2000 != 0 {
        s.push(if mode & 0o010 != 0 { 's' } else { 'S' });
    } else {
        s.push(if mode & 0o010 != 0 { 'x' } else { '-' });
    }

    // Others
    s.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    s.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    if mode & 0o1000 != 0 {
        s.push(if mode & 0o001 != 0 { 't' } else { 'T' });
    } else {
        s.push(if mode & 0o001 != 0 { 'x' } else { '-' });
    }

    s
}

/// Get file owner name from UID.
fn uid_to_name(uid: u32) -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                if let Ok(file_uid) = parts[2].parse::<u32>() {
                    if file_uid == uid {
                        return parts[0].to_string();
                    }
                }
            }
        }
    }
    uid.to_string()
}

/// Get group name from GID.
fn gid_to_name(gid: u32) -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/group") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                if let Ok(file_gid) = parts[2].parse::<u32>() {
                    if file_gid == gid {
                        return parts[0].to_string();
                    }
                }
            }
        }
    }
    gid.to_string()
}

/// Stat a file path and return PermFileInfo using libc::stat.
fn stat_file(path: &str, is_suid_scan: bool) -> Option<PermFileInfo> {
    use std::ffi::CString;

    let c_path = CString::new(path).ok()?;

    unsafe {
        let mut stat_buf: libc::stat = std::mem::zeroed();
        if libc::stat(c_path.as_ptr(), &mut stat_buf) != 0 {
            return None;
        }

        let mode = stat_buf.st_mode as u32;
        let uid = stat_buf.st_uid;
        let gid = stat_buf.st_gid;
        let size = stat_buf.st_size as u64;

        let owner = uid_to_name(uid);
        let group_name = gid_to_name(gid);
        let perms = mode_to_string(mode);
        let size_text = format_size(size);

        let risk = if is_suid_scan {
            // SUID root = high, SGID or SUID non-root = medium, else low
            if mode & 0o4000 != 0 && uid == 0 {
                "high".to_string()
            } else if mode & 0o4000 != 0 || mode & 0o2000 != 0 {
                "medium".to_string()
            } else {
                "low".to_string()
            }
        } else {
            // World-writable: all medium-risk
            "medium".to_string()
        };

        Some(PermFileInfo {
            path: path.to_string(),
            owner,
            group_name,
            perms,
            size_text,
            risk,
        })
    }
}

/// Scan for SUID/SGID files.
fn scan_suid_files(state: &Arc<Mutex<PermState>>) {
    let output = cmd_output(
        "sh",
        &["-c", "find / -maxdepth 5 -perm /6000 -type f 2>/dev/null | head -200"],
    );

    let mut files: Vec<PermFileInfo> = Vec::new();
    for path in output.lines().take(200) {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if let Some(info) = stat_file(path, true) {
            files.push(info);
        }
    }

    // Sort by risk: high first
    files.sort_by(|a, b| {
        let risk_ord = |r: &str| match r {
            "high" => 0,
            "medium" => 1,
            _ => 2,
        };
        risk_ord(&a.risk)
            .cmp(&risk_ord(&b.risk))
            .then(a.path.cmp(&b.path))
    });

    if let Ok(mut s) = state.lock() {
        s.suid_files = files;
        s.dirty = true;
    }
}

/// Scan for world-writable files.
fn scan_world_writable(state: &Arc<Mutex<PermState>>) {
    let output = cmd_output(
        "sh",
        &["-c", "find / -maxdepth 4 -perm -o+w -not -path '/proc/*' -not -path '/sys/*' -not -path '/dev/*' 2>/dev/null | head -200"],
    );

    let mut files: Vec<PermFileInfo> = Vec::new();
    for path in output.lines().take(200) {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if let Some(info) = stat_file(path, false) {
            files.push(info);
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));

    if let Ok(mut s) = state.lock() {
        s.world_writable = files;
        s.dirty = true;
    }
}

/// Refresh all permission data.
fn refresh_all(state: &Arc<Mutex<PermState>>) {
    // Users and groups are fast (file parse only)
    let users = parse_users();
    let groups = parse_groups();

    if let Ok(mut s) = state.lock() {
        s.users = users;
        s.groups = groups;
        s.scanning = true;
        s.dirty = true;
    }

    // Filesystem scans are slow
    scan_suid_files(state);
    scan_world_writable(state);

    if let Ok(mut s) = state.lock() {
        s.scanning = false;
        s.dirty = true;
    }
}

/// Sync state to UI properties.
fn sync_to_ui(ui: &App, state: &Arc<Mutex<PermState>>) {
    let snap = match state.lock() {
        Ok(mut s) => {
            if !s.dirty {
                return;
            }
            s.dirty = false;
            PermSnapshot {
                users: s.users.clone(),
                groups: s.groups.clone(),
                suid_files: s.suid_files.clone(),
                world_writable: s.world_writable.clone(),
                scanning: s.scanning,
            }
        }
        Err(_) => return,
    };

    // Users
    let user_items: Vec<UserData> = snap
        .users
        .iter()
        .map(|u| UserData {
            username: u.username.clone().into(),
            uid: u.uid,
            gid: u.gid,
            home: u.home.clone().into(),
            shell: u.shell.clone().into(),
            is_system: u.is_system,
        })
        .collect();
    ui.set_perm_users(ModelRc::new(VecModel::from(user_items)));

    // Groups
    let group_items: Vec<GroupData> = snap
        .groups
        .iter()
        .map(|g| GroupData {
            name: g.name.clone().into(),
            gid: g.gid,
            member_count: g.member_count,
            members: g.members.clone().into(),
        })
        .collect();
    ui.set_perm_groups(ModelRc::new(VecModel::from(group_items)));

    // SUID/SGID files
    let suid_items: Vec<PermFileData> = snap
        .suid_files
        .iter()
        .map(|f| PermFileData {
            path: f.path.clone().into(),
            owner: f.owner.clone().into(),
            group_name: f.group_name.clone().into(),
            perms: f.perms.clone().into(),
            size_text: f.size_text.clone().into(),
            risk: f.risk.clone().into(),
        })
        .collect();
    ui.set_perm_suid_files(ModelRc::new(VecModel::from(suid_items)));

    // World-writable files
    let ww_items: Vec<PermFileData> = snap
        .world_writable
        .iter()
        .map(|f| PermFileData {
            path: f.path.clone().into(),
            owner: f.owner.clone().into(),
            group_name: f.group_name.clone().into(),
            perms: f.perms.clone().into(),
            size_text: f.size_text.clone().into(),
            risk: f.risk.clone().into(),
        })
        .collect();
    ui.set_perm_world_writable(ModelRc::new(VecModel::from(ww_items)));

    // Scanning state
    ui.set_perm_scanning(snap.scanning);
}

struct PermSnapshot {
    users: Vec<UserInfo>,
    groups: Vec<GroupInfo>,
    suid_files: Vec<PermFileInfo>,
    world_writable: Vec<PermFileInfo>,
    scanning: bool,
}

/// Wire the Permission Dashboard callbacks and timers.
pub fn wire(ui: &crate::App, _ctx: &crate::app_context::AppContext) {
    let state = Arc::new(Mutex::new(PermState::new()));

    // Initial refresh in background
    {
        let state_clone = state.clone();
        std::thread::spawn(move || {
            refresh_all(&state_clone);
        });
    }

    // 30-second refresh timer
    let refresh_timer = Timer::default();
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        refresh_timer.start(
            TimerMode::Repeated,
            std::time::Duration::from_secs(30),
            move || {
                // Sync to UI on timer tick
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &state_clone);
                }

                // Trigger background refresh
                let state_bg = state_clone.clone();
                std::thread::spawn(move || {
                    refresh_all(&state_bg);
                });
            },
        );
    }
    std::mem::forget(refresh_timer);

    // Fast initial sync timer (200ms repeated to catch first data)
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        let init_timer = Timer::default();
        init_timer.start(
            TimerMode::Repeated,
            std::time::Duration::from_millis(200),
            move || {
                if let Some(ui) = ui_weak.upgrade() {
                    sync_to_ui(&ui, &state_clone);
                }
            },
        );
        std::mem::forget(init_timer);
    }

    // ── Callbacks ──

    // Tab switch
    ui.on_perm_tab(move |_index| {
        // Tab switch is handled in slint (active-tab property).
        // This callback exists for any future backend logic on tab change.
    });

    // Refresh — trigger a full rescan
    {
        let state_clone = state.clone();
        ui.on_perm_refresh(move || {
            let st = state_clone.clone();
            if let Ok(mut s) = st.lock() {
                s.scanning = true;
                s.dirty = true;
            }
            std::thread::spawn(move || {
                refresh_all(&st);
            });
        });
    }

    // Fix world-writable permissions (chmod o-w)
    {
        let state_clone = state.clone();
        ui.on_perm_fix(move |path| {
            let path_str = path.to_string();
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("chmod")
                    .args(["o-w", &path_str])
                    .output();
                // Re-scan world-writable after fix
                scan_world_writable(&st);
            });
        });
    }

    // Select user (just updates the selected index in UI)
    {
        let ui_weak = ui.as_weak();
        ui.on_perm_select_user(move |index| {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_perm_selected_user(index);
            }
        });
    }
}
