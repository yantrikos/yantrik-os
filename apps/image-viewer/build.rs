fn main() {
    // The app's screen is its own (ui/image_viewer.slint); the kit and the theme come through the
    // framework's build.
    yantrik_app_build::UiBuild::new().compile("ui/app.slint");
}
