//! Compact tool-schema compiler for small models.
//!
//! Small models (Tiny/Small tiers) pay a heavy token cost for full JSON tool
//! schemas — a tool with a long description and a dozen properties costs
//! hundreds of tokens, and that block is the whole tool surface a 0.5B model
//! sees. This module compiles one tool definition down to a few lines of plain
//! text: name, a one-sentence description, and one line per parameter. JSON
//! Schema noise (`additionalProperties`, `$schema`, `items` sub-schemas beyond
//! the item type, long `enum` lists) is dropped.

use crate::capability::ModelTier;
use serde_json::Value;

/// Character / count budgets for the compact tool form.
#[derive(Debug, Clone, Copy)]
pub struct ToolBudget {
    /// Max characters for the tool description.
    pub desc_chars: usize,
    /// Max characters for each parameter description.
    pub param_desc_chars: usize,
    /// Max number of parameters emitted.
    pub max_params: usize,
}

impl ToolBudget {
    /// Budget for a capability tier. Tiny/Small get the tight form;
    /// Medium/Large keep the full JSON schema (see [`ToolBudget::is_compact`]).
    pub fn for_tier(tier: ModelTier) -> Self {
        match tier {
            ModelTier::Tiny => ToolBudget { desc_chars: 100, param_desc_chars: 40, max_params: 5 },
            ModelTier::Small => ToolBudget { desc_chars: 160, param_desc_chars: 60, max_params: 8 },
            // Medium/Large keep the full schema; these are only used if a
            // caller forces the compact form.
            ModelTier::Medium => ToolBudget { desc_chars: 400, param_desc_chars: 160, max_params: 24 },
            ModelTier::Large => ToolBudget { desc_chars: usize::MAX, param_desc_chars: usize::MAX, max_params: usize::MAX },
        }
    }

    /// Whether this tier should use the compact form instead of full JSON.
    pub fn is_compact(tier: ModelTier) -> bool {
        matches!(tier, ModelTier::Tiny | ModelTier::Small)
    }
}

/// First sentence of `s`, then truncated to `max` chars on a char boundary.
pub(crate) fn first_sentence(s: &str, max: usize) -> String {
    let s = s.trim();
    let cut = match s.find(". ") {
        Some(pos) => &s[..pos + 1],
        None => s,
    };
    if cut.chars().count() <= max {
        cut.to_string()
    } else {
        let mut out: String = cut.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Compact type label for a parameter: its `type` (arrays surface the item
/// type as `array<T>`), plus a short enum list capped at 4 values.
fn param_type(field: &Value) -> String {
    let base = match field.get("type").and_then(|t| t.as_str()) {
        Some("array") => match field.get("items").and_then(|i| i.get("type")).and_then(|t| t.as_str()) {
            Some(it) => format!("array<{}>", it),
            None => "array".to_string(),
        },
        Some(t) => t.to_string(),
        None => "string".to_string(),
    };

    if let Some(vals) = field.get("enum").and_then(|e| e.as_array()) {
        let shown: Vec<String> = vals.iter().take(4).map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }).collect();
        let mut label = format!("{}[{}", base, shown.join("|"));
        if vals.len() > 4 {
            label.push_str("|…");
        }
        label.push(']');
        return label;
    }
    base
}

/// Parameters in compact order: those named in `required` first (in
/// `required` order), then the rest in declaration order, capped at `max` so
/// an over-parameterized tool keeps the arguments it cannot do without. The cap
/// only drops optional parameters: a call missing a required one cannot succeed.
pub(crate) fn ordered_params<'a>(
    props: &'a serde_json::Map<String, Value>,
    required: &[&str],
    max: usize,
) -> Vec<(&'a String, &'a Value)> {
    let mut ordered: Vec<(&String, &Value)> = Vec::with_capacity(props.len().min(max));
    for req in required {
        if let Some((k, v)) = props.get_key_value(*req) {
            ordered.push((k, v));
        }
    }
    let kept = ordered.len().max(max);
    for (k, v) in props {
        if !required.iter().any(|r| *r == k.as_str()) {
            ordered.push((k, v));
        }
    }
    ordered.truncate(kept);
    ordered
}

/// Compile one tool definition to compact text.
///
/// Reads `function.name`/`description`/`parameters` first, falling back to the
/// top-level keys (MCP tools arrive in both shapes). Required parameters are
/// emitted first and marked with `*`.
pub fn compact_tool(def: &Value, budget: ToolBudget) -> String {
    let func = def.get("function").unwrap_or(def);

    let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
    let desc = func.get("description").and_then(|d| d.as_str()).unwrap_or("");

    let mut out = String::new();
    out.push_str(name);
    if !desc.is_empty() {
        out.push_str(": ");
        out.push_str(&first_sentence(desc, budget.desc_chars));
    }

    let params = func.get("parameters");
    let props = params.and_then(|p| p.get("properties")).and_then(|p| p.as_object());
    let required: Vec<&str> = params
        .and_then(|p| p.get("required"))
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if let Some(map) = props {
        for (pname, field) in ordered_params(map, &required, budget.max_params) {
            let is_req = required.iter().any(|r| *r == pname.as_str());
            out.push('\n');
            out.push_str(pname);
            if is_req {
                out.push('*');
            }
            out.push('(');
            out.push_str(&param_type(field));
            out.push(')');
            let pdesc = field.get("description").and_then(|d| d.as_str()).unwrap_or("");
            if !pdesc.is_empty() {
                out.push_str(": ");
                out.push_str(&first_sentence(pdesc, budget.param_desc_chars));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A definition with a 400-char description, 8 properties each with a
    /// 200-char description, `additionalProperties: false` and a 10-value enum.
    fn fat_tool() -> Value {
        let long_desc = "x".repeat(400);
        let long_param = "y".repeat(200);
        let mut props = serde_json::Map::new();
        for i in 0..8 {
            props.insert(format!("p{}", i), json!({
                "type": "string",
                "description": long_param,
                "enum": ["a","b","c","d","e","f","g","h","i","j"],
            }));
        }
        json!({
            "type": "function",
            "function": {
                "name": "do_thing",
                "description": long_desc,
                "parameters": {
                    "type": "object",
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "additionalProperties": false,
                    "properties": props,
                    "required": ["p0", "p3"],
                },
            },
        })
    }

    #[test]
    fn small_tier_gets_compact_text() {
        let json_form = serde_json::to_string(&fat_tool()).unwrap();
        for tier in [ModelTier::Tiny, ModelTier::Small] {
            let body = compact_tool(&fat_tool(), ToolBudget::for_tier(tier));
            assert!(!body.contains("additionalProperties"), "{tier:?}: compact body leaks additionalProperties");
            assert!(!body.contains("$schema"), "{tier:?}: compact body leaks $schema");
            // At most 4 enum values survive, plus the "…" marker.
            assert!(!body.contains("|e|"), "{tier:?}: compact body leaks a 5th enum value");
            assert!(body.contains("…"), "{tier:?}: truncated enum should carry an ellipsis");
            // Materially shorter than the JSON form.
            assert!(body.len() * 3 < json_form.len(), "{tier:?}: compact {} vs json {}", body.len(), json_form.len());
        }
        // The tightest budget keeps one fat tool under a hard character cap.
        let tiny = compact_tool(&fat_tool(), ToolBudget::for_tier(ModelTier::Tiny));
        assert!(tiny.len() < 600, "Tiny per-tool body too long: {} chars", tiny.len());
    }

    #[test]
    fn name_and_required_params_survive() {
        let body = compact_tool(&fat_tool(), ToolBudget::for_tier(ModelTier::Tiny));
        assert!(body.contains("do_thing"), "tool name must survive");
        assert!(body.contains("p0*"), "required param p0 must be named and marked");
        assert!(body.contains("p3*"), "required param p3 must be named and marked");
        // Required params come first: p0 appears before p1.
        let p0 = body.find("p0").unwrap();
        let p1 = body.find("p1").unwrap();
        assert!(p0 < p1, "required parameters must be emitted first");
    }

    #[test]
    fn top_level_shape_is_handled() {
        // MCP tools can arrive without the `function` wrapper.
        let def = json!({
            "name": "flat_tool",
            "description": "Does a thing. More detail here.",
            "parameters": {
                "type": "object",
                "properties": { "q": { "type": "string", "description": "Query string" } },
                "required": ["q"],
            },
        });
        let body = compact_tool(&def, ToolBudget::for_tier(ModelTier::Small));
        assert!(body.contains("flat_tool"));
        assert!(body.contains("Does a thing."));
        assert!(body.contains("q*(string): Query string"));
    }

    #[test]
    fn enum_capped_at_four() {
        let def = json!({
            "function": {
                "name": "t",
                "description": "d",
                "parameters": {
                    "properties": { "c": { "type": "string", "enum": ["1","2","3","4","5","6"] } },
                },
            },
        });
        let body = compact_tool(&def, ToolBudget::for_tier(ModelTier::Small));
        assert!(body.contains("[1|2|3|4|…"), "got: {}", body);
        assert!(!body.contains("5"));
    }

    #[test]
    fn the_param_cap_never_drops_a_required_param() {
        let mut props = serde_json::Map::new();
        let names: Vec<String> = (0..7).map(|i| format!("r{}", i)).collect();
        for n in &names {
            props.insert(n.clone(), json!({ "type": "string" }));
        }
        props.insert("opt".into(), json!({ "type": "string" }));
        let def = json!({
            "function": {
                "name": "t",
                "parameters": { "properties": props, "required": names },
            },
        });
        // Tiny keeps at most 5 parameters, but all 7 are required.
        let body = compact_tool(&def, ToolBudget::for_tier(ModelTier::Tiny));
        for i in 0..7 {
            assert!(body.contains(&format!("r{}*", i)), "required r{} dropped: {}", i, body);
        }
        assert!(!body.contains("opt"), "the optional one is what the cap drops: {}", body);
    }
}
