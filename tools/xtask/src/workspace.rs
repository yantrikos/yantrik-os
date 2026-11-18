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
    /// The other members whose `testing` feature its own default build turns on (see
    /// [`ships_testing`]).
    pub ships_testing: Vec<String>,
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
            ships_testing: ships_testing(p, name, &names),
        });
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(out)
}

/// The members whose `testing` feature `package` turns on in a build that is not a test: a normal
/// or build dependency that asks for it, or the default features reaching `<member>/testing`
/// (through other features of its own, too). Dev-dependencies are a test's own and do not count.
/// `cargo build --workspace` merges every member's features into one build, so one such member
/// puts the test-only API into every binary built beside it.
fn ships_testing(package: &serde_json::Value, name: &str, names: &[&str]) -> Vec<String> {
    let deps = package["dependencies"].as_array().map(Vec::as_slice).unwrap_or_default();
    let member = |d: &str| names.contains(&d) && d != name;
    let mut out: Vec<String> = deps
        .iter()
        .filter(|d| d["kind"].as_str() != Some("dev") && d["optional"].as_bool() != Some(true))
        .filter(|d| d["features"].as_array().is_some_and(|f| f.iter().any(|f| f == "testing")))
        .filter_map(|d| d["name"].as_str())
        .filter(|d| member(d))
        .map(str::to_string)
        .collect();
    // The default features, followed through the package's own features.
    let features = &package["features"];
    let mut seen: Vec<&str> = Vec::new();
    let mut todo = vec!["default"];
    while let Some(feature) = todo.pop() {
        if seen.contains(&feature) {
            continue;
        }
        seen.push(feature);
        for entry in features[feature].as_array().into_iter().flatten().filter_map(|e| e.as_str()) {
            match entry.split_once('/') {
                Some((dep, "testing")) if member(dep.trim_end_matches('?')) => {
                    out.push(dep.trim_end_matches('?').to_string());
                }
                Some(_) => {}
                None if features.get(entry).is_some() => todo.push(entry),
                None => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
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
                uses: vec!["kit".into()],
                ships_testing: Vec::new(),
            }
        );
        assert_eq!(m[1].dir, "crates/kit");
        assert!(m[1].uses.is_empty());
    }

    #[test]
    fn a_testing_feature_turned_on_outside_the_tests_is_seen() {
        let json = r#"{"workspace_root":"/w","packages":[
            {"name":"harness","manifest_path":"/w/f/harness/Cargo.toml","dependencies":[
                {"name":"core","kind":null,"features":["testing"]}],"features":{}},
            {"name":"app","manifest_path":"/w/apps/app/Cargo.toml","dependencies":[
                {"name":"core","kind":"dev","features":["testing"]},
                {"name":"harness","kind":"dev","features":["testing"]},
                {"name":"kit","kind":"build","features":["testing"]},
                {"name":"serde","kind":null,"features":["testing"]}],"features":{}},
            {"name":"tool","manifest_path":"/w/tools/tool/Cargo.toml","dependencies":[
                {"name":"core","kind":null,"features":[]},
                {"name":"kit","kind":null,"features":[],"optional":true}],
             "features":{"default":["extra"],"extra":["core?/testing","kit/other"],"fakes":["kit/testing"]}},
            {"name":"core","manifest_path":"/w/f/core/Cargo.toml","dependencies":[],
             "features":{"testing":[]}},
            {"name":"kit","manifest_path":"/w/f/kit/Cargo.toml","dependencies":[],"features":{}}]}"#;
        let m = parse(json).unwrap();
        let ships = |name: &str| m.iter().find(|m| m.name == name).unwrap().ships_testing.clone();
        assert_eq!(ships("harness"), vec!["core"], "a normal dependency that asks for it");
        assert_eq!(ships("app"), vec!["kit"], "a build dependency counts; a dev one and a non-member do not");
        assert_eq!(ships("tool"), vec!["core"], "the default features, followed; `fakes` is not default");
        assert!(ships("core").is_empty());
    }
}
