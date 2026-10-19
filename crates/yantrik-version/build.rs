//! Bakes the build-time version into the crate, so a binary carries an answer even when it is
//! running outside an installed tree.
//!
//! `git describe --tags --match "v[0-9]*" --always --dirty` is the same command `build-release.sh` runs to name
//! the tarball and fill the BUILD marker, and `build-debian-iso.sh` runs to name the ISO. One
//! command, one string, whichever of them produced the thing you are looking at.
//!
//! `YANTRIK_VERSION` wins when it is set, because CI computes the string once and hands the
//! same one to every step; a build that recomputed it could land on a different answer than
//! the tarball it goes into (a tag pushed between two steps is enough).

fn main() {
    // A build script's cwd is this crate's directory, which is inside the repository when the
    // OS is built from source, and is not when the crate is vendored. Both are fine: an
    // unavailable git leaves the variable unset and the runtime falls back.
    println!("cargo:rerun-if-env-changed=YANTRIK_VERSION");
    for path in git_watch_paths() {
        println!("cargo:rerun-if-changed={path}");
    }

    if let Some(v) = std::env::var("YANTRIK_VERSION").ok().filter(|v| !v.trim().is_empty()) {
        println!("cargo:rustc-env=YANTRIK_BUILD_VERSION={}", v.trim());
        return;
    }

    if let Some(v) = git_describe() {
        println!("cargo:rustc-env=YANTRIK_BUILD_VERSION={v}");
    }
}

fn git_describe() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["describe", "--tags", "--match", "v[0-9]*", "--always", "--dirty"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// The files whose change means `git describe` would now answer differently: where HEAD points,
/// and the refs and tags it is named against.
///
/// Asked of git rather than assumed: in a worktree `.git` is a file, HEAD belongs to the
/// worktree's own directory, and the refs are in the common one. Only paths that exist are
/// emitted — cargo treats a `rerun-if-changed` on a missing path as "always rerun", and this
/// crate is a dependency of the shell and every app, so an always-dirty build script here would
/// relink the whole OS on every build.
fn git_watch_paths() -> Vec<String> {
    let git_dir = git_path("--git-dir");
    let common = git_path("--git-common-dir");
    [
        git_dir.as_ref().map(|d| format!("{d}/HEAD")),
        common.as_ref().map(|d| format!("{d}/refs")),
        common.as_ref().map(|d| format!("{d}/packed-refs")),
    ]
    .into_iter()
    .flatten()
    .filter(|p| std::path::Path::new(p).exists())
    .collect()
}

fn git_path(which: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", which])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
