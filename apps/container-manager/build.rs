fn main() {
    // The app's screen is its own (ui/container_manager.slint); the kit and the theme come
    // through the framework's build.
    yantrik_app_build::UiBuild::new().compile("ui/app.slint");
}
