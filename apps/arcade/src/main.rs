fn main() -> std::process::ExitCode {
    // A headless command (`build`, `verify`, `screenshot`) opens no window and does not fight the
    // running app for its instance lock: it answers first, as it always did.
    if let Some(code) = yantrik_arcade::cli::run(std::env::args().collect()) {
        return std::process::ExitCode::from(code.clamp(0, 255) as u8);
    }
    yantrik_app::run::<yantrik_arcade::Arcade>()
}
