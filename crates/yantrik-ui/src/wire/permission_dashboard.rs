//! Permission Dashboard wire module — users, groups, SUID/SGID, world-writable.
//!
//! Parses /etc/passwd and /etc/group for user/group data. Runs `find` commands
//! in background threads to locate SUID/SGID and world-writable files.
//! 30-second refresh interval for filesystem scans.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

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
    numeric_perms: String,
    owner_rwx: String,
    group_rwx: String,
    other_rwx: String,
    recommended_perms: String,
    // Pre-computed booleans for UI grid
    owner_r: bool,
    owner_w: bool,
    owner_x: bool,
    group_r: bool,
    group_w: bool,
    group_x: bool,
    other_r: bool,
    other_w: bool,
    other_x: bool,
}

/// Shared permission state — updated by background threads, read by UI timer.
struct PermState {
    users: Vec<UserInfo>,
    groups: Vec<GroupInfo>,
    suid_files: Vec<PermFileInfo>,
    world_writable: Vec<PermFileInfo>,
    /// Filtered copies for display (after search)
    filtered_suid: Vec<PermFileInfo>,
    filtered_ww: Vec<PermFileInfo>,
    scanning: bool,
    dirty: bool,
    scan_path: String,
    scan_status: String,
    search_query: String,
    selected_file: Option<PermFileInfo>,
    export_status: String,
    high_risk: i32,
    medium_risk: i32,
    low_risk: i32,
}

impl PermState {
    fn new() -> Self {
        Self {
            users: Vec::new(),
            groups: Vec::new(),
            suid_files: Vec::new(),
            world_writable: Vec::new(),
            filtered_suid: Vec::new(),
            filtered_ww: Vec::new(),
            scanning: false,
            dirty: true,
            scan_path: "/".to_string(),
            scan_status: String::new(),
            search_query: String::new(),
            selected_file: None,
            export_status: String::new(),
            high_risk: 0,
            medium_risk: 0,
            low_risk: 0,
        }
    }

    /// Recompute risk counts from both file lists.
    fn recompute_risk_counts(&mut self) {
        let (mut h, mut m, mut l) = (0i32, 0i32, 0i32);
        for f in self.suid_files.iter().chain(self.world_writable.iter()) {
            match f.risk.as_str() {
                "high" => h += 1,
                "medium" => m += 1,
                _ => l += 1,
            }
        }
        self.high_risk = h;
        self.medium_risk = m;
        self.low_risk = l;
    }

    /// Apply search filter to produce filtered lists.
    fn apply_filter(&mut self) {
        let q = self.search_query.to_lowercase();
        if q.is_empty() {
            self.filtered_suid = self.suid_files.clone();
            self.filtered_ww = self.world_writable.clone();
        } else {
            self.filtered_suid = self
                .suid_files
                .iter()
                .filter(|f| {
                    f.path.to_lowercase().contains(&q)
                        || f.risk.to_lowercase().contains(&q)
                        || f.owner.to_lowercase().contains(&q)
                })
                .cloned()
                .collect();
            self.filtered_ww = self
                .world_writable
                .iter()
                .filter(|f| {
                    f.path.to_lowercase().contains(&q)
                        || f.risk.to_lowercase().contains(&q)
                        || f.owner.to_lowercase().contains(&q)
                })
                .cloned()
                .collect();
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

/// Extract owner rwx (3 chars) from the full rwx string.
fn extract_owner_rwx(perms: &str) -> String {
    if perms.len() >= 4 {
        perms[1..4].to_string()
    } else {
        "---".to_string()
    }
}

/// Extract group rwx (3 chars) from the full rwx string.
fn extract_group_rwx(perms: &str) -> String {
    if perms.len() >= 7 {
        perms[4..7].to_string()
    } else {
        "---".to_string()
    }
}

/// Extract other rwx (3 chars) from the full rwx string.
fn extract_other_rwx(perms: &str) -> String {
    if perms.len() >= 10 {
        perms[7..10].to_string()
    } else {
        "---".to_string()
    }
}

/// Convert mode to octal string like "0755".
fn mode_to_octal(mode: u32) -> String {
    format!("{:04o}", mode & 0o7777)
}

/// Compute recommended safe permissions for a file based on its current mode.
fn recommended_perms(mode: u32, is_suid: bool) -> String {
    if is_suid {
        // For SUID files: recommend removing SUID bit if not root-owned common util
        let base = mode & 0o0777;
        // Recommend keeping base permissions but removing SUID/SGID
        if mode & 0o4000 != 0 || mode & 0o2000 != 0 {
            return format!("{:04o}", base);
        }
    }

    // World-writable: remove world-write bit
    if mode & 0o002 != 0 {
        let fixed = mode & !0o002 & 0o7777;
        return format!("{:04o}", fixed);
    }

    String::new()
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
        let numeric_perms = mode_to_octal(mode);
        let owner_rwx = extract_owner_rwx(&perms);
        let group_rwx = extract_group_rwx(&perms);
        let other_rwx = extract_other_rwx(&perms);

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

        let rec = recommended_perms(mode, is_suid_scan);

        // Pre-compute boolean flags for rwx grid
        let owner_r = mode & 0o400 != 0;
        let owner_w = mode & 0o200 != 0;
        let owner_x = mode & 0o100 != 0 || mode & 0o4000 != 0;
        let group_r = mode & 0o040 != 0;
        let group_w = mode & 0o020 != 0;
        let group_x = mode & 0o010 != 0 || mode & 0o2000 != 0;
        let other_r = mode & 0o004 != 0;
        let other_w = mode & 0o002 != 0;
        let other_x = mode & 0o001 != 0 || mode & 0o1000 != 0;

        Some(PermFileInfo {
            path: path.to_string(),
            owner,
            group_name,
            perms,
            size_text,
            risk,
            numeric_perms,
            owner_rwx,
            group_rwx,
            other_rwx,
            recommended_perms: rec,
            owner_r,
            owner_w,
            owner_x,
            group_r,
            group_w,
            group_x,
            other_r,
            other_w,
            other_x,
        })
    }
}

/// Scan for SUID/SGID files under a given path.
fn scan_suid_files(state: &Arc<Mutex<PermState>>, scan_path: &str) {
    let find_cmd = format!(
        "find {} -maxdepth 5 -perm /6000 -type f 2>/dev/null | head -200",
        shell_escape(scan_path)
    );
    let output = cmd_output("sh", &["-c", &find_cmd]);

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
        s.apply_filter();
        s.recompute_risk_counts();
        s.dirty = true;
    }
}

/// Scan for world-writable files under a given path.
fn scan_world_writable(state: &Arc<Mutex<PermState>>, scan_path: &str) {
    let find_cmd = format!(
        "find {} -maxdepth 4 -perm -o+w -not -path '/proc/*' -not -path '/sys/*' -not -path '/dev/*' 2>/dev/null | head -200",
        shell_escape(scan_path)
    );
    let output = cmd_output("sh", &["-c", &find_cmd]);

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
        s.apply_filter();
        s.recompute_risk_counts();
        s.dirty = true;
    }
}

/// Simple shell-escape for a path: wrap in single quotes.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Refresh all permission data.
fn refresh_all(state: &Arc<Mutex<PermState>>) {
    let scan_path = state
        .lock()
        .map(|s| s.scan_path.clone())
        .unwrap_or_else(|_| "/".to_string());

    // Users and groups are fast (file parse only)
    let users = parse_users();
    let groups = parse_groups();

    if let Ok(mut s) = state.lock() {
        s.users = users;
        s.groups = groups;
        s.scanning = true;
        s.scan_status = "Scanning...".to_string();
        s.dirty = true;
    }

    // Filesystem scans are slow
    scan_suid_files(state, &scan_path);
    scan_world_writable(state, &scan_path);

    if let Ok(mut s) = state.lock() {
        s.scanning = false;
        let total = s.suid_files.len() + s.world_writable.len();
        s.scan_status = format!("Done: {} files scanned", total);
        s.dirty = true;
    }
}

/// Convert PermFileInfo to Slint PermFileData.
fn to_slint_file(f: &PermFileInfo) -> PermFileData {
    PermFileData {
        path: f.path.clone().into(),
        owner: f.owner.clone().into(),
        group_name: f.group_name.clone().into(),
        perms: f.perms.clone().into(),
        size_text: f.size_text.clone().into(),
        risk: f.risk.clone().into(),
        numeric_perms: f.numeric_perms.clone().into(),
        owner_rwx: f.owner_rwx.clone().into(),
        group_rwx: f.group_rwx.clone().into(),
        other_rwx: f.other_rwx.clone().into(),
        recommended_perms: f.recommended_perms.clone().into(),
        owner_r: f.owner_r,
        owner_w: f.owner_w,
        owner_x: f.owner_x,
        group_r: f.group_r,
        group_w: f.group_w,
        group_x: f.group_x,
        other_r: f.other_r,
        other_w: f.other_w,
        other_x: f.other_x,
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
                filtered_suid: s.filtered_suid.clone(),
                filtered_ww: s.filtered_ww.clone(),
                scanning: s.scanning,
                scan_status: s.scan_status.clone(),
                selected_file: s.selected_file.clone(),
                export_status: s.export_status.clone(),
                high_risk: s.high_risk,
                medium_risk: s.medium_risk,
                low_risk: s.low_risk,
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

    // SUID/SGID files (filtered)
    let suid_items: Vec<PermFileData> = snap.filtered_suid.iter().map(to_slint_file).collect();
    ui.set_perm_suid_files(ModelRc::new(VecModel::from(suid_items)));

    // World-writable files (filtered)
    let ww_items: Vec<PermFileData> = snap.filtered_ww.iter().map(to_slint_file).collect();
    ui.set_perm_world_writable(ModelRc::new(VecModel::from(ww_items)));

    // Scanning state
    ui.set_perm_scanning(snap.scanning);

    // Risk counts
    ui.set_perm_high_risk_count(snap.high_risk);
    ui.set_perm_medium_risk_count(snap.medium_risk);
    ui.set_perm_low_risk_count(snap.low_risk);

    // Scan status
    ui.set_perm_scan_status(snap.scan_status.into());

    // Selected file detail
    if let Some(ref f) = snap.selected_file {
        ui.set_perm_selected_file(to_slint_file(f));
        ui.set_perm_has_selected_file(true);
    } else {
        ui.set_perm_has_selected_file(false);
    }

    // Export status
    ui.set_perm_export_status(snap.export_status.into());
}

struct PermSnapshot {
    users: Vec<UserInfo>,
    groups: Vec<GroupInfo>,
    filtered_suid: Vec<PermFileInfo>,
    filtered_ww: Vec<PermFileInfo>,
    scanning: bool,
    scan_status: String,
    selected_file: Option<PermFileInfo>,
    export_status: String,
    high_risk: i32,
    medium_risk: i32,
    low_risk: i32,
}

/// Wire the Permission Dashboard callbacks and timers.
pub fn wire(ui: &crate::App, ctx: &crate::app_context::AppContext) {
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
                let scan_path = st
                    .lock()
                    .map(|s| s.scan_path.clone())
                    .unwrap_or_else(|_| "/".to_string());
                // Re-scan world-writable after fix
                scan_world_writable(&st, &scan_path);
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

    // Select file — populate detail panel
    {
        let state_clone = state.clone();
        ui.on_perm_select_file(move |tab, idx| {
            if let Ok(mut s) = state_clone.lock() {
                let file = match tab {
                    1 => s.filtered_suid.get(idx as usize).cloned(),
                    2 => s.filtered_ww.get(idx as usize).cloned(),
                    _ => None,
                };
                s.selected_file = file;
                s.dirty = true;
            }
        });
    }

    // Scan directory — targeted scan of a specific path
    {
        let state_clone = state.clone();
        ui.on_perm_scan_dir(move |path| {
            let path_str = path.to_string();
            let st = state_clone.clone();
            if let Ok(mut s) = st.lock() {
                s.scan_path = path_str.clone();
                s.scanning = true;
                s.scan_status = format!("Scanning {}...", path_str);
                s.dirty = true;
            }
            std::thread::spawn(move || {
                refresh_all(&st);
            });
        });
    }

    // Export report
    {
        let state_clone = state.clone();
        ui.on_perm_export_report(move || {
            let st = state_clone.clone();
            std::thread::spawn(move || {
                let report = {
                    let s = match st.lock() {
                        Ok(s) => s,
                        Err(_) => return,
                    };

                    let mut buf = String::new();
                    buf.push_str("# Yantrik OS Permission Scan Report\n");
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    buf.push_str(&format!("# Generated: timestamp {}\n", now));
                    buf.push_str(&format!("# Scan path: {}\n\n", s.scan_path));

                    buf.push_str(&format!(
                        "## Risk Summary\n  High: {}  Medium: {}  Low: {}\n\n",
                        s.high_risk, s.medium_risk, s.low_risk
                    ));

                    if !s.suid_files.is_empty() {
                        buf.push_str("## SUID/SGID Files\n");
                        buf.push_str(&format!(
                            "{:<6} {:<50} {:<10} {:<12} {:<8} {}\n",
                            "Risk", "Path", "Owner", "Perms", "Mode", "Size"
                        ));
                        buf.push_str(&"-".repeat(96));
                        buf.push('\n');
                        for f in &s.suid_files {
                            buf.push_str(&format!(
                                "{:<6} {:<50} {:<10} {:<12} {:<8} {}\n",
                                f.risk.to_uppercase(),
                                f.path,
                                f.owner,
                                f.perms,
                                f.numeric_perms,
                                f.size_text
                            ));
                        }
                        buf.push('\n');
                    }

                    if !s.world_writable.is_empty() {
                        buf.push_str("## World-Writable Files\n");
                        buf.push_str(&format!(
                            "{:<50} {:<10} {:<12} {:<8} {}\n",
                            "Path", "Owner", "Perms", "Mode", "Size"
                        ));
                        buf.push_str(&"-".repeat(90));
                        buf.push('\n');
                        for f in &s.world_writable {
                            buf.push_str(&format!(
                                "{:<50} {:<10} {:<12} {:<8} {}\n",
                                f.path, f.owner, f.perms, f.numeric_perms, f.size_text
                            ));
                        }
                    }

                    buf
                };

                // Write report to /tmp
                let report_path = "/tmp/yantrik_permission_report.txt";
                match std::fs::write(report_path, &report) {
                    Ok(_) => {
                        if let Ok(mut s) = st.lock() {
                            s.export_status = format!("Exported to {}", report_path);
                            s.dirty = true;
                        }
                    }
                    Err(e) => {
                        if let Ok(mut s) = st.lock() {
                            s.export_status = format!("Export failed: {}", e);
                            s.dirty = true;
                        }
                    }
                }

                // Clear export status after 5 seconds
                std::thread::sleep(std::time::Duration::from_secs(5));
                if let Ok(mut s) = st.lock() {
                    s.export_status = String::new();
                    s.dirty = true;
                }
            });
        });
    }

    // Search / filter
    {
        let state_clone = state.clone();
        ui.on_perm_search(move |query| {
            if let Ok(mut s) = state_clone.lock() {
                s.search_query = query.to_string();
                s.apply_filter();
                // Clear selected file when search changes
                s.selected_file = None;
                s.dirty = true;
            }
        });
    }

    // Fix permission — apply recommended permission change
    {
        let state_clone = state.clone();
        ui.on_perm_fix_permission(move |path| {
            let path_str = path.to_string();
            let st = state_clone.clone();

            // Get the recommended permission from state
            let rec_perms = if let Ok(s) = st.lock() {
                s.selected_file
                    .as_ref()
                    .map(|f| f.recommended_perms.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            if rec_perms.is_empty() {
                return;
            }

            std::thread::spawn(move || {
                let _ = std::process::Command::new("chmod")
                    .args([&rec_perms, &path_str])
                    .output();

                // Clear selection and rescan
                if let Ok(mut s) = st.lock() {
                    s.selected_file = None;
                    s.scanning = true;
                    s.scan_status = "Re-scanning after fix...".to_string();
                    s.dirty = true;
                }
                refresh_all(&st);
            });
        });
    }

    // ── View ACL callback ──
    {
        let ui_weak = ui.as_weak();
        ui.on_perm_view_acl(move |path| {
            let path_str = path.to_string();
            let ui_w = ui_weak.clone();
            std::thread::spawn(move || {
                let output = cmd_output("getfacl", &[&path_str]);
                let result = if output.is_empty() {
                    format!("No ACL data for {}\n(getfacl may not be installed)", path_str)
                } else {
                    output
                };
                let _ = ui_w.upgrade_in_event_loop(move |ui| {
                    ui.set_perm_acl_output(result.into());
                });
            });
        });
    }

    // ── Set ACL callback ──
    {
        let ui_weak = ui.as_weak();
        ui.on_perm_set_acl(move |path, spec| {
            let path_str = path.to_string();
            let spec_str = spec.to_string();
            let ui_w = ui_weak.clone();
            std::thread::spawn(move || {
                let output = std::process::Command::new("setfacl")
                    .args(["-m", &spec_str, &path_str])
                    .output();

                let result = match output {
                    Ok(o) if o.status.success() => {
                        // Re-read ACL after setting
                        let new_acl = cmd_output("getfacl", &[&path_str]);
                        if new_acl.is_empty() {
                            "ACL applied successfully.".to_string()
                        } else {
                            format!("ACL applied.\n\n{}", new_acl)
                        }
                    }
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                        format!("setfacl failed: {}", stderr.trim())
                    }
                    Err(e) => format!("Failed to run setfacl: {}", e),
                };
                let _ = ui_w.upgrade_in_event_loop(move |ui| {
                    ui.set_perm_acl_output(result.into());
                });
            });
        });
    }

    // ── AI Explain callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_perm_ai_explain(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Gather security-relevant data from UI
        let users = ui.get_perm_users();
        let suid = ui.get_perm_suid_files();
        let world_w = ui.get_perm_world_writable();

        let mut context = format!(
            "Users: {} total\nSUID files: {} found\nWorld-writable files: {} found\n",
            users.row_count(), suid.row_count(), world_w.row_count()
        );

        // List a few SUID files
        if suid.row_count() > 0 {
            context.push_str("\nSUID files:\n");
            for i in 0..suid.row_count().min(10) {
                if let Some(f) = suid.row_data(i) {
                    context.push_str(&format!("  {}\n", f.path));
                }
            }
        }

        let prompt = super::ai_assist::permission_analysis_prompt(&context);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_perm_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_perm_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_perm_ai_response().to_string()),
            },
        );
    });

    let ui_weak = ui.as_weak();
    ui.on_perm_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_perm_ai_panel_open(false);
        }
    });
}
