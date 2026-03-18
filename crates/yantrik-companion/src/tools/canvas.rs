//! Canvas tools — natural language diagram generation via Graphviz.
//!
//! Uses LLM to generate DOT source, renders via `dot` command to PNG.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

/// Register canvas tools.
pub fn register(reg: &mut ToolRegistry, ollama_base: &str, model: &str) {
    let base = ollama_base.trim_end_matches('/').to_string();
    let mdl = model.to_string();
    reg.register(Box::new(GenerateDiagramTool { ollama_base: base.clone(), model: mdl.clone() }));
    reg.register(Box::new(EditDiagramTool { ollama_base: base, model: mdl }));
}

/// Ask LLM to generate DOT source for a diagram.
fn llm_generate_dot(ollama_base: &str, model: &str, description: &str, style: &str) -> Result<String, String> {
    let system_prompt = format!(
        r##"You are a diagram expert. Generate Graphviz DOT source code for the requested diagram.
Style: {style}
Rules:
- Output ONLY valid DOT code, no explanation
- Use readable labels
- For flowcharts: use rankdir=TB, rounded rectangles, arrows with labels
- For mindmaps: use rankdir=LR, colorful nodes
- For sequence diagrams: simulate with subgraphs and edges
- For ER diagrams: use record-shaped nodes
- Use a dark theme: bgcolor="#1a1a2e", fontcolor="white", node color="#16213e", edge color="#5ac8d4"
"##
    );

    let payload = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": format!("Generate a {style} diagram for: {description}")}
        ],
        "stream": false
    });

    let payload_path = "/tmp/yantrik-canvas-payload.json";
    std::fs::write(payload_path, payload.to_string())
        .map_err(|e| format!("Failed to write payload: {e}"))?;

    let url = format!("{}/api/chat", ollama_base);

    let output = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "60", "-H", "Content-Type: application/json", "-d", &format!("@{payload_path}"), &url])
        .output()
        .map_err(|e| format!("curl failed: {e}"))?;

    let _ = std::fs::remove_file(payload_path);

    if !output.status.success() {
        return Err(format!("LLM request failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let response: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid JSON: {e}"))?;

    let content = response["message"]["content"]
        .as_str()
        .ok_or("No content in response")?;

    // Extract DOT code from markdown code blocks if present
    let dot = if content.contains("```") {
        content.split("```")
            .nth(1)
            .unwrap_or(content)
            .trim_start_matches("dot")
            .trim_start_matches("graphviz")
            .trim()
            .to_string()
    } else {
        content.trim().to_string()
    };

    Ok(dot)
}

/// Render DOT source to PNG using graphviz.
fn render_dot(dot_source: &str) -> Result<String, String> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let out_path = format!("/tmp/yantrik-diagram-{ts}.png");
    let dot_path = format!("/tmp/yantrik-diagram-{ts}.dot");

    std::fs::write(&dot_path, dot_source)
        .map_err(|e| format!("Failed to write DOT file: {e}"))?;

    let output = std::process::Command::new("dot")
        .args(["-Tpng", "-Gdpi=150", "-o", &out_path, &dot_path])
        .output()
        .map_err(|e| format!("graphviz not available: {e}"))?;

    let _ = std::fs::remove_file(&dot_path);

    if !output.status.success() {
        return Err(format!("dot rendering failed: {}", String::from_utf8_lossy(&output.stderr)));
    }

    Ok(out_path)
}

// ── generate_diagram ──

pub struct GenerateDiagramTool {
    ollama_base: String,
    model: String,
}

impl Tool for GenerateDiagramTool {
    fn name(&self) -> &'static str { "generate_diagram" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "canvas" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "generate_diagram",
                "description": "Generate a diagram from a natural language description",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "description": {
                            "type": "string",
                            "description": "What to diagram (e.g. 'user authentication flow', 'microservice architecture')"
                        },
                        "style": {
                            "type": "string",
                            "enum": ["flowchart", "mindmap", "sequence", "class", "er"],
                            "description": "Diagram style (default: flowchart)"
                        }
                    },
                    "required": ["description"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let description = match args.get("description").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => return "Error: description parameter required".to_string(),
        };
        let style = args.get("style").and_then(|v| v.as_str()).unwrap_or("flowchart");

        let dot_source = match llm_generate_dot(&self.ollama_base, &self.model, description, style) {
            Ok(dot) => dot,
            Err(e) => return format!("Failed to generate diagram: {e}"),
        };

        let image_path = match render_dot(&dot_source) {
            Ok(path) => path,
            Err(e) => return format!("Failed to render diagram: {e}\n\nDOT source:\n{dot_source}"),
        };

        serde_json::json!({
            "status": "success",
            "image_path": image_path,
            "dot_source": dot_source,
            "description": description,
            "style": style
        }).to_string()
    }
}

// ── edit_diagram ──

pub struct EditDiagramTool {
    ollama_base: String,
    model: String,
}

impl Tool for EditDiagramTool {
    fn name(&self) -> &'static str { "edit_diagram" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "canvas" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_diagram",
                "description": "Edit an existing diagram by applying natural language",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "edit": {
                            "type": "string",
                            "description": "What to change (e.g. 'add a database step after login', 'make the colors warmer')"
                        },
                        "dot_source": {
                            "type": "string",
                            "description": "The current DOT source code to modify"
                        }
                    },
                    "required": ["edit", "dot_source"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let edit = match args.get("edit").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return "Error: edit parameter required".to_string(),
        };
        let dot_source = match args.get("dot_source").and_then(|v| v.as_str()) {
            Some(d) => d,
            None => return "Error: dot_source parameter required".to_string(),
        };

        // Ask LLM to modify the existing DOT source
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "You are a Graphviz expert. Modify the given DOT source code according to the user's request. Output ONLY the modified DOT code, no explanation."},
                {"role": "user", "content": format!("Current DOT source:\n```\n{dot_source}\n```\n\nEdit: {edit}")}
            ],
            "stream": false
        });

        let payload_path = "/tmp/yantrik-canvas-edit-payload.json";
        if let Err(e) = std::fs::write(payload_path, payload.to_string()) {
            return format!("Failed to write payload: {e}");
        }

        let url = format!("{}/api/chat", self.ollama_base);

        let output = match std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", "-H", "Content-Type: application/json", "-d", &format!("@{payload_path}"), &url])
            .output() {
            Ok(o) => o,
            Err(e) => { let _ = std::fs::remove_file(payload_path); return format!("curl failed: {e}"); }
        };

        let _ = std::fs::remove_file(payload_path);

        if !output.status.success() {
            return format!("LLM request failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        let response: serde_json::Value = match serde_json::from_slice(&output.stdout) {
            Ok(r) => r,
            Err(e) => return format!("Invalid JSON: {e}"),
        };

        let content = match response["message"]["content"].as_str() {
            Some(c) => c,
            None => return "No content in LLM response".to_string(),
        };

        // Extract DOT from code blocks
        let new_dot = if content.contains("```") {
            content.split("```")
                .nth(1)
                .unwrap_or(content)
                .trim_start_matches("dot")
                .trim_start_matches("graphviz")
                .trim()
                .to_string()
        } else {
            content.trim().to_string()
        };

        let image_path = match render_dot(&new_dot) {
            Ok(path) => path,
            Err(e) => return format!("Failed to render edited diagram: {e}\n\nDOT source:\n{new_dot}"),
        };

        serde_json::json!({
            "status": "success",
            "image_path": image_path,
            "dot_source": new_dot,
            "edit_applied": edit
        }).to_string()
    }
}
