fn main() {
    // The shared theme (colours, Barlow) and the kit's lock view, so the session lock is drawn by the
    // same component as the shell's own lock screen.
    yantrik_app_build::UiBuild::new().compile("ui/lock.slint");
}
