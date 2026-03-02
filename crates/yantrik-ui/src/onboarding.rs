//! First-boot onboarding — marker file + guided Lens results.

use std::path::PathBuf;

use slint::SharedString;

use super::LensResult;

/// Path to the onboarding completion marker file.
pub fn marker_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".yantrik").join(".onboarding_complete")
}

/// Write the onboarding completion marker.
pub fn write_marker() {
    let path = marker_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, "done");
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
