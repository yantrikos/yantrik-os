//! ProjectConsciousness — auto-detect projects and track context.
//!
//! Listens for file change events to detect project directories,
//! generates urges on project switches, tracks time per project.

use super::{FeatureContext, Outcome, ProactiveFeature, Urge, UrgeCategory};
use yantrik_os::SystemEvent;

pub struct ProjectConsciousness {
    active_project: Option<ProjectInfo>,
    tick_count: u32,
}

#[derive(Clone, Debug)]
struct ProjectInfo {
    name: String,
    path: String,
    project_type: String,
}

impl ProjectConsciousness {
    pub fn new() -> Self {
        Self {
            active_project: None,
            tick_count: 0,
        }
    }
}

impl ProactiveFeature for ProjectConsciousness {
    fn name(&self) -> &str { "ProjectConsciousness" }

    fn on_event(&mut self, event: &SystemEvent, _ctx: &FeatureContext) -> Vec<Urge> {
        // Detect projects from file change events — if a file is modified
        // in a project directory, that project becomes active.
        if let SystemEvent::FileChanged { path, .. } = event {
            if let Some(parent) = std::path::Path::new(path).parent() {
                let dir = parent.to_string_lossy();
                if let Some(project) = detect_project(&dir) {
                    let switched = match &self.active_project {
                        Some(prev) => prev.path != project.path,
                        None => true,
                    };

                    if switched {
                        let urge = Urge {
                            id: format!("project-switch-{}", project.name),
                            source: "ProjectConsciousness".to_string(),
                            title: format!("Project: {}", project.name),
                            body: format!(
                                "Working on {} ({} project)",
                                project.name, project.project_type
                            ),
                            urgency: 0.2,
                            confidence: 0.8,
                            category: UrgeCategory::Project,
                        };
                        self.active_project = Some(project);
                        return vec![urge];
                    }
                }
            }
        }

        Vec::new()
    }

    fn on_tick(&mut self, _ctx: &FeatureContext) -> Vec<Urge> {
        self.tick_count += 1;
        Vec::new()
    }

    fn on_feedback(&mut self, _urge_id: &str, _outcome: Outcome) {}
}

/// Detect a project by walking up from cwd looking for project markers.
fn detect_project(cwd: &str) -> Option<ProjectInfo> {
    let mut path = std::path::PathBuf::from(cwd);

    for _ in 0..10 {
        // Rust project
        if path.join("Cargo.toml").exists() {
            let name = extract_cargo_name(&path).unwrap_or_else(|| dir_name(&path));
            return Some(ProjectInfo {
                name,
                path: path.to_string_lossy().to_string(),
                project_type: "rust".to_string(),
            });
        }

        // Node.js project
        if path.join("package.json").exists() {
            let name = extract_package_name(&path).unwrap_or_else(|| dir_name(&path));
            return Some(ProjectInfo {
                name,
                path: path.to_string_lossy().to_string(),
                project_type: "node".to_string(),
            });
        }

        // Python project
        if path.join("pyproject.toml").exists() || path.join("setup.py").exists() {
            return Some(ProjectInfo {
                name: dir_name(&path),
                path: path.to_string_lossy().to_string(),
                project_type: "python".to_string(),
            });
        }

        // Go project
        if path.join("go.mod").exists() {
            return Some(ProjectInfo {
                name: dir_name(&path),
                path: path.to_string_lossy().to_string(),
                project_type: "go".to_string(),
            });
        }

        // Generic git repo (lowest priority)
        if path.join(".git").exists() {
            return Some(ProjectInfo {
                name: dir_name(&path),
                path: path.to_string_lossy().to_string(),
                project_type: "git".to_string(),
            });
        }

        if !path.pop() {
            break;
        }
    }

    None
}

fn dir_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

fn extract_cargo_name(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path.join("Cargo.toml")).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("name") && trimmed.contains('=') {
            let val = trimmed.split('=').nth(1)?.trim().trim_matches('"');
            return Some(val.to_string());
        }
    }
    None
}

fn extract_package_name(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("name")?.as_str().map(|s| s.to_string())
}
