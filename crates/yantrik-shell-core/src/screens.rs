//! Screen ID constants and app_id → screen routing.
//!
//! Single source of truth for screen IDs used by navigation, dock, and command palette.

/// Screen IDs — matches the `current-screen` property in app.slint.
pub mod id {
    pub const BOOT: i32 = 0;
    pub const DESKTOP: i32 = 1;
    pub const ONBOARDING: i32 = 2;
    pub const LOCK: i32 = 3;
    pub const BOND: i32 = 4;
    pub const PERSONALITY: i32 = 5;
    pub const MEMORY: i32 = 6;
    pub const SETTINGS: i32 = 7;
    pub const FILES: i32 = 8;
    pub const NOTIFICATIONS: i32 = 9;
    pub const SYSTEM_DASHBOARD: i32 = 10;
    pub const IMAGE_VIEWER: i32 = 11;
    pub const TEXT_EDITOR: i32 = 12;
    pub const MEDIA_PLAYER: i32 = 13;
    pub const TERMINAL: i32 = 14;
    pub const NOTES: i32 = 15;
    pub const ABOUT: i32 = 16;
    pub const EMAIL: i32 = 17;
    pub const CALENDAR: i32 = 18;
    pub const WEATHER: i32 = 19;
    pub const MUSIC_PLAYER: i32 = 20;
    pub const PACKAGE_MANAGER: i32 = 21;
    pub const NETWORK_MANAGER: i32 = 22;
    pub const SYSTEM_MONITOR: i32 = 23;
    pub const DOWNLOAD_MANAGER: i32 = 24;
    pub const SNIPPET_MANAGER: i32 = 25;
    pub const CONTAINER_MANAGER: i32 = 26;
    pub const DEVICE_DASHBOARD: i32 = 27;
    pub const PERMISSION_DASHBOARD: i32 = 28;
    pub const SPREADSHEET: i32 = 29;
    pub const DOCUMENT_EDITOR: i32 = 30;
    pub const PRESENTATION: i32 = 31;
    pub const SKILL_STORE: i32 = 32;
}

/// Map a built-in app_id to its screen ID. Returns None for external apps.
pub fn screen_for_app(app_id: &str) -> Option<i32> {
    match app_id {
        "terminal" => Some(id::TERMINAL),
        "files" => Some(id::FILES),
        "email" => Some(id::EMAIL),
        "notes" => Some(id::NOTES),
        "editor" => Some(id::TEXT_EDITOR),
        "media" => Some(id::MEDIA_PLAYER),
        "bond" => Some(id::BOND),
        "personality" => Some(id::PERSONALITY),
        "memory" => Some(id::MEMORY),
        "notifications" => Some(id::NOTIFICATIONS),
        "system" => Some(id::SYSTEM_DASHBOARD),
        "settings" => Some(id::SETTINGS),
        "about" => Some(id::ABOUT),
        "packages" => Some(id::PACKAGE_MANAGER),
        "network" => Some(id::NETWORK_MANAGER),
        "sysmonitor" => Some(id::SYSTEM_MONITOR),
        "weather" => Some(id::WEATHER),
        "music" => Some(id::MUSIC_PLAYER),
        "downloads" => Some(id::DOWNLOAD_MANAGER),
        "snippets" => Some(id::SNIPPET_MANAGER),
        "containers" => Some(id::CONTAINER_MANAGER),
        "devices" => Some(id::DEVICE_DASHBOARD),
        "permissions" => Some(id::PERMISSION_DASHBOARD),
        "calendar" => Some(id::CALENDAR),
        "spreadsheet" => Some(id::SPREADSHEET),
        "documents" => Some(id::DOCUMENT_EDITOR),
        "presentation" => Some(id::PRESENTATION),
        "skills" | "skill_store" => Some(id::SKILL_STORE),
        _ => None,
    }
}
