//! First-boot onboarding — marker file, guided Lens results, and profile setup.
//!
//! During onboarding, the user selects interests, sets their location,
//! and picks a notification preference. These are stored as:
//! - User interests → companion `user_interests` field + PWG Interest nodes
//! - Location → companion `user_location` + config
//! - Notification preference → config `proactive.mode`

use std::path::PathBuf;

use slint::SharedString;

use super::LensResult;

/// Path to the onboarding completion marker file.
pub fn marker_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".yantrik").join(".onboarding_complete")
}

/// Where the installer leaves word that it created this account, relative to the home
/// directory (#400). The installer asks only what installing needs; the first boot then offers
/// the rest — interests, how to be reached, the AI — as optional setup, opening on a welcome
/// with Skip instead of asking for the person's name again.
pub const AFTER_INSTALL_MARKER: &str = ".yantrik/.setup_after_install";

/// The installer's marker for this account.
pub fn after_install_marker_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(AFTER_INSTALL_MARKER)
}

/// Write the onboarding completion marker.
pub fn write_marker() {
    let path = marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "done");
    // Its job is done: the setup it asked for was finished or skipped.
    let _ = std::fs::remove_file(after_install_marker_path());
}

/// How long a machine is new. Before a day has passed since the first run there is no day behind
/// it to brief on or recap: 13 Oct, minutes after setup, the Lens offered "Evening review: Recap
/// today's activity".
pub const NEW_FOR: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Whether the person has a history here worth a brief or a review: the first run ended at least
/// [`NEW_FOR`] ago. A machine whose first run has not ended has none.
pub fn has_history(since_first_run: Option<std::time::Duration>) -> bool {
    since_first_run.is_some_and(|age| age >= NEW_FOR)
}

/// How long ago the first run ended, from its marker's time.
pub fn since_first_run() -> Option<std::time::Duration> {
    std::fs::metadata(marker_path()).ok()?.modified().ok()?.elapsed().ok()
}

/// Generate the guided Lens result for the current onboarding step.
pub fn guide_result(step: i32) -> LensResult {
    match step {
        1 => LensResult {
            result_type: "guide".into(),
            title: "Try: open a terminal".into(),
            subtitle: "Your first command — launch an app".into(),
            icon_char: "▶".into(),
            action_id: "launch:terminal".into(),
            score: 0.0,
            is_loading: false,
            inline_value: SharedString::default(),
        },
        2 => LensResult {
            result_type: "guide".into(),
            title: "Try: set a 5 min focus timer".into(),
            subtitle: "Watch the desktop transform".into(),
            icon_char: "◎".into(),
            action_id: "setting:focus:300".into(),
            score: 0.0,
            is_loading: false,
            inline_value: SharedString::default(),
        },
        _ => LensResult {
            result_type: "guide".into(),
            title: "You're all set".into(),
            subtitle: "Explore freely".into(),
            icon_char: "✓".into(),
            action_id: "".into(),
            score: 0.0,
            is_loading: false,
            inline_value: SharedString::default(),
        },
    }
}

/// Parsed onboarding profile data from the UI.
#[derive(Debug, Clone)]
pub struct OnboardingProfile {
    /// Selected interest categories.
    pub interests: Vec<String>,
    /// Home location string (city/region).
    pub home_location: String,
    /// Notification preference: "morning_brief", "realtime", "minimal".
    pub notification_pref: String,
}

/// Parse the interest string from the UI (comma-separated, trailing comma).
pub fn parse_profile(interests_csv: &str, home_location: &str, notif_pref: &str) -> OnboardingProfile {
    let interests: Vec<String> = interests_csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    OnboardingProfile {
        interests,
        home_location: home_location.trim().to_string(),
        notification_pref: notif_pref.trim().to_string(),
    }
}

/// Save the onboarding profile to the config file on disk.
/// Appends/updates interest, location, and notification sections.
pub fn save_profile_to_config(profile: &OnboardingProfile, config_path: &str) {
    // Read existing config
    let content = std::fs::read_to_string(config_path).unwrap_or_default();

    // Build the profile section
    let mut additions = String::new();

    // Add interests
    if !profile.interests.is_empty() {
        // Check if user_interests already exists
        if !content.contains("user_interests:") {
            additions.push_str("\n# User interests (from onboarding)\nuser_interests:\n");
            for interest in &profile.interests {
                additions.push_str(&format!("  - \"{}\"\n", interest));
            }
        }
    }

    // Add location
    if !profile.home_location.is_empty() && !content.contains("user_location:") {
        additions.push_str(&format!(
            "\n# User location (from onboarding)\nuser_location: \"{}\"\n",
            profile.home_location,
        ));
    }

    // Add notification preference
    if !content.contains("notification_mode:") {
        additions.push_str(&format!(
            "\n# Notification preference (from onboarding)\nnotification_mode: \"{}\"\n",
            profile.notification_pref,
        ));
    }

    if !additions.is_empty() {
        let updated = format!("{}{}", content, additions);
        if let Err(e) = std::fs::write(config_path, updated) {
            tracing::warn!("Failed to save onboarding profile to config: {e}");
        } else {
            tracing::info!(
                interests = profile.interests.len(),
                location = %profile.home_location,
                notif = %profile.notification_pref,
                "Onboarding profile saved to config"
            );
        }
    }
}
