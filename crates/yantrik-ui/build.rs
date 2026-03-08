fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    // Embed build date
    let date = chrono_lite_date();
    println!("cargo:rustc-env=BUILD_DATE={date}");

    // Embed git commit hash (short)
    if let Some(hash) = git_short_hash() {
        println!("cargo:rustc-env=GIT_HASH={hash}");
    }

    // Embed component versions from workspace crate Cargo.toml files
    let components = [
        ("YANTRIK_ML", "../yantrik-ml"),
        ("YANTRIKDB_CORE", "../yantrikdb-core"),
        ("YANTRIK_COMPANION", "../yantrik-companion"),
        ("YANTRIK_OS", "../yantrik-os"),
        ("YANTRIK_UI", "."),
    ];

    for (env_name, crate_path) in &components {
        let toml_path = format!("{}/Cargo.toml", crate_path);
        if let Some(ver) = read_cargo_version(&toml_path) {
            println!("cargo:rustc-env=COMPONENT_{env_name}_VERSION={ver}");
        }
        // Also embed git hash for each component's repo (if it's a separate repo via patch)
        if let Some(hash) = git_short_hash_at(crate_path) {
            println!("cargo:rustc-env=COMPONENT_{env_name}_GIT={hash}");
        }
    }
}

fn chrono_lite_date() -> String {
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

fn git_short_hash_at(path: &str) -> Option<String> {
    // All crates are in the same workspace repo, so they share the same git hash
    // But in production builds they may come from separate repos via git deps
    git_short_hash()
}

fn read_cargo_version(toml_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(toml_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("version") && trimmed.contains('=') {
            // version = "0.1.0"
            let val = trimmed.split('=').nth(1)?.trim();
            let ver = val.trim_matches('"').trim_matches('\'');
            return Some(ver.to_string());
        }
    }
    None
}
