//! Compact tool-schema compiler for small models.
//!
//! A full JSON Schema for a tool can cost hundreds of tokens — far too much
//! for a 0.5–4B model that only needs the name, a one-line description, and
//! the parameter names/types. This module turns one tool definition into a
//! short text form, chosen by model tier (#452).

use crate::capability::ModelTier;

/// Per-tool truncation budget for the compact form.
#[derive(Debug, Clone, Copy)]
pub struct ToolBudget {
    /// Max characters for the tool description.
    pub desc_chars: usize,
    /// Max characters for each parameter description.
    pub param_desc_chars: usize,
    /// Max parameters to emit.
    pub max_params: usize,
}

impl ToolBudget {
    /// Pick a budget for a model tier. Tiny/Small get the tight budget;
    /// Medium/Large keep something close to today's full text.
    pub fn for_tier(tier: &ModelTier) -> Self {
        match tier {
            ModelTier::Tiny | ModelTier::Small => ToolBudget {
                desc_chars: 160,
                param_desc_chars: 60,
                max_params: 12,
            },
            ModelTier::Medium | ModelTier::Large => ToolBudget {
                desc_chars: 400,
                param_desc_chars: 200,
                max_params: 64,
            },
        }
    }
}

/// Cut a description to its first sentence, then to at most `max` chars.
fn first_sentence(desc: &str, max: usize) -> &str {
    let desc = desc.trim();
    let first = if let Some(pos) = desc.find(". ") {
        &desc[..pos]
    } else {
        desc
    };
    if first.len() > max {
        &first[..first.floor_char_boundary(max)]
    } else {
        first
    }
}

/// Turn one tool definition into compact text.
///
/// Reads `def["function"]["name"]`, `def["function"]["description"]` and
/// `def["function"]["parameters"]["properties"]`, falling back to the
/// top-level `name` / `description` / `parameters` keys (MCP tools arrive in
/// both shapes). Required parameters come first and are marked with `*`.
pub fn compact_tool(def: &serde_json::Value, budget: ToolBudget) -> String {
    let func = def.get("function").unwrap_or(def);
    let name = func
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("unknown");
    let desc = func
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("");

    let mut s = String::new();
    s.push_str(name);
    if !desc.is_empty() {
        s.push_str(": ");
        s.push_str(first_sentence(desc, budget.desc_chars));
    }

    let params = func.get("parameters").unwrap_or(func);
    let props = params.get("properties").and_then(|p| p.as_object());
    let required: Vec<&str> = params
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if let Some(props) = props {
        // Required parameters first, then the rest, capped by max_params.
        let mut ordered: Vec<(&String, &serde_json::Value, bool)> = Vec::new();
        for (pname, pfields) in props {
            let is_req = required.iter().any(|r| r == pname);
            ordered.push((pname, pfields, is_req));
        }
        ordered.sort_by_key(|(_, _, is_req)| !is_req);

        for (pname, pfields, is_req) in ordered.into_iter().take(budget.max_params) {
            let ptype = pfields
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string");
            let pdesc = pfields
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");

            s.push_str("\n  ");
            if is_req {
                s.push('*');
            }
            s.push_str(pname);
            s.push('(');
            s.push_str(ptype);
            s.push_str(")");
            if !pdesc.is_empty() {
                s.push_str(": ");
                s.push_str(first_sentence(pdesc, budget.param_desc_chars));
            }
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_def() -> serde_json::Value {
        let mut desc = String::new();
        for _ in 0..20 {
            desc.push_str("This is a long tool description sentence. ");
        }
        let mut props = serde_json::Map::new();
        for i in 0..8 {
            let mut pdesc = String::new();
            for _ in 0..10 {
                pdesc.push_str("A long parameter description sentence. ");
            }
            props.insert(
                format!("param_{i}"),
                serde_json::json!({
                    "type": "string",
                    "description": pdesc,
                }),
            );
        }
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "sample_tool",
                "description": desc,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": ["param_0", "param_1"],
                    "additionalProperties": false,
                }
            }
        })
    }

    #[test]
    fn small_tier_gets_compact_text() {
        let def = sample_def();
        let budget = ToolBudget::for_tier(&ModelTier::Small);
        let text = compact_tool(&def, budget);

        assert!(!text.contains("additionalProperties"));
        assert!(!text.contains("$schema"));
        assert!(text.len() < 600, "compact body too long: {} chars", text.len());
        assert!(text.contains("sample_tool"));
        assert!(text.contains("*param_0"));
        assert!(text.contains("*param_1"));
    }

    #[test]
    fn name_and_required_params_survive() {
        let def = sample_def();
        let budget = ToolBudget::for_tier(&ModelTier::Tiny);
        let text = compact_tool(&def, budget);

        assert!(text.contains("sample_tool"));
        for i in 0..8 {
            assert!(text.contains(&format!("param_{i}")), "missing param_{i}");
        }
        assert!(text.contains("*param_0"));
        assert!(text.contains("*param_1"));
    }
}
