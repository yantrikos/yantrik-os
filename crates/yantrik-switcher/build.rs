fn main() {
    // The shared theme (colours, Barlow), so Alt+Tab is drawn in the shell's own look, and the
    // UI kit, for AppTile: each window's icon is the one the dock draws.
    yantrik_app_build::UiBuild::new().compile("ui/switcher.slint");
    println!("cargo:rerun-if-changed=../../packaging/labwc/labwc-window-switcher-renderer-unstable-v1.xml");
}
