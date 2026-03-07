fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    // Embed build date
    let date = chrono_lite_date();
    println!("cargo:rustc-env=BUILD_DATE={date}");

    // Embed git commit hash (short)
    if let Some(hash) = git_short_hash() {
        println!("cargo:rustc-env=GIT_HASH={hash}");
    }
}

fn chrono_lite_date() -> String {
    // Use chrono-free approach: just call `date` or fall back to compile date
    std::process::Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_short_hash() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}
