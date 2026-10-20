//! The workspace as the checks see it: its members, their folders and which members they use.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One workspace member.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub name: String,
    /// Its folder relative to the workspace root, `/`-separated.
    pub dir: String,
    /// The other members it depends on, of any kind (normal, build or dev).
    pub uses: Vec<String>,
}

/// The workspace root: two folders above this crate.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root")
}

/// The members, from `cargo metadata` (without resolving dependencies, so it needs no network).
pub fn members(root: &Path) -> Result<Vec<Member>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cargo metadata did not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// The members in `cargo metadata` output.
pub fn parse(json: &str) -> Result<Vec<Member>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("cargo metadata output: {e}"))?;
    let root = slashed(
        v["workspace_root"]
            .as_str()
            .ok_or("cargo metadata has no workspace_root")?,
    );
    let packages = v["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?;
    let names: Vec<&str> = packages.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in packages {
        let name = p["name"].as_str().ok_or("a package without a name")?;
        let manifest = slashed(
            p["manifest_path"]
                .as_str()
                .ok_or("a package without a manifest")?,
        );
        let dir = manifest
            .strip_suffix("/Cargo.toml")
            .and_then(|d| d.strip_prefix(&root))
            .map(|d| d.trim_start_matches('/').to_string())
            .ok_or_else(|| format!("{name}: its manifest {manifest} is not under {root}"))?;
        let mut uses: Vec<String> = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["name"].as_str())
            .filter(|d| names.contains(d) && *d != name)
            .map(str::to_string)
            .collect();
        uses.sort();
        uses.dedup();
        out.push(Member {
            name: name.to_string(),
            dir,
            uses,
        });
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(out)
}

fn slashed(path: &str) -> String {
    path.replace('\\', "/")
}

/// The files git tracks under `root` that match `patterns` (git pathspecs), `/`-separated.
pub fn tracked(root: &Path, patterns: &[&str]) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("ls-files")
        .arg("-z")
        .arg("--")
        .args(patterns)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git ls-files did not run: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_and_what_they_use() {
        let json = r#"{"workspace_root":"C:\\w","packages":[
            {"name":"app","manifest_path":"C:\\w\\apps\\app\\Cargo.toml","dependencies":[
                {"name":"kit"},{"name":"serde"},{"name":"kit"},{"name":"app"}]},
            {"name":"kit","manifest_path":"C:\\w\\crates\\kit\\Cargo.toml","dependencies":[]}]}"#;
        let m = parse(json).unwrap();
        assert_eq!(
            m[0],
            Member {
                name: "app".into(),
                dir: "apps/app".into(),
                uses: vec!["kit".into()]
            }
        );
        assert_eq!(m[1].dir, "crates/kit");
        assert!(m[1].uses.is_empty());
    }
}
