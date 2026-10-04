//! The colour roles, held where a sweep of VM 520 (4 Oct 2026) found them broken.
//!
//! Teal is the minds' colour and nothing else's; amber is "needs you" and nothing else; a
//! primary button is the accent in every app (design/minds-surfaces-spec-2026-10-02.md,
//! "Colour roles"). These read the source, so they run without building Slint.

use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const UI: &str = "crates/yantrik-ui-slint/ui/";

/// Screens that are not a mind's, and so must not draw the minds' teal. Containers' "Run New",
/// About's version pill and section bars, the Skills chips, the System and System Monitor bars,
/// the setup and login screens, and the shared chart, toast and ribbon all did.
#[test]
fn teal_stays_off_screens_that_are_not_a_minds() {
    for file in [
        "container_manager.slint", "about.slint", "skill_store.slint", "settings.slint",
        "system_dashboard.slint", "system_monitor.slint", "recipes.slint", "permission_dashboard.slint",
        "installer.slint", "onboarding.slint", "login.slint", "terminal.slint",
        "components/onboard_kit.slint", "components/app_ribbon.slint", "components/window_frame.slint",
        "components/model_tier_badge.slint",
    ] {
        let src = read(&format!("{UI}{file}"));
        for teal in ["Theme.cyan", "Theme.tint-cyan"] {
            assert!(!src.contains(teal), "{file} draws `{teal}`, the minds' colour");
        }
    }
    for kit in ["line_chart.slint", "toast_banner.slint"] {
        assert!(!read(&format!("crates/yantrik-ui-kit/slint/{kit}")).contains("Theme.cyan"), "{kit} defaults to the minds' teal");
    }
}

/// A primary that wears the app's identity colour is teal in Containers and red in Permissions
/// ("Scanning..." looked destructive). These screens use the kit's YButton now.
#[test]
fn a_primary_button_is_the_accent_not_the_apps_colour() {
    for file in ["container_manager.slint", "permission_dashboard.slint", "network_manager.slint"] {
        let src = read(&format!("{UI}{file}"));
        assert!(!src.contains("primary ? AppIdentity.accent"), "{file} fills its primary with the app's colour");
        assert!(src.contains("import { YButton }"), "{file} uses the kit's button");
    }
    let perm = read(&format!("{UI}permission_dashboard.slint"));
    let scan = perm.find("\"Scanning...\" : \"Scan\"").expect("the Scan button");
    assert!(perm[scan..scan + 120].contains("variant: 0;"), "Scan is the accent primary, never the destructive kind");
}

/// Amber means a person's answer is pending. A bond score, a busy CPU and a model tier are not.
#[test]
fn amber_is_not_used_for_data() {
    for file in ["components/bond_ring.slint", "system_dashboard.slint", "system_monitor.slint", "components/model_tier_badge.slint"] {
        let src = read(&format!("{UI}{file}"));
        assert!(!src.contains("Theme.amber;") && !src.contains("? Theme.amber :"), "{file} paints data amber");
    }
}

/// A finished recipe step is not a success in teal (the success token's dark value is a teal):
/// done recedes to neutral, the accent marks the current step, amber the person's turn.
#[test]
fn a_done_recipe_step_is_neutral() {
    let src = read(&format!("{UI}recipes.slint"));
    assert!(!src.contains("color-success"), "recipes.slint draws done in the success teal");
}
