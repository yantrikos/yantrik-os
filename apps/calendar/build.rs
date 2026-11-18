fn main() {
    // The app's screen is still compiled from the shell's folder; migration step 3 moves it here.
    yantrik_app_build::UiBuild::new()
        .include_dir("../../crates/yantrik-ui-slint/ui")
        .compile("ui/app.slint");
}
