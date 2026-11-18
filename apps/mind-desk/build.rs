fn main() {
    // The desk's screen is its own (ui/desk.slint); the kit and the theme come through the
    // framework's build.
    yantrik_app_build::UiBuild::new().compile("ui/app.slint");
}
