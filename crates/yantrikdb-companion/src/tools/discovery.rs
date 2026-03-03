//! Tool discovery meta-tool — lets the LLM browse available tools
//! without having all schemas injected upfront.
//!
//! Always included in the system prompt. Returns compact metadata
//! (name | category | permission | description) so the LLM can
//! identify the right tool, then call it in the next round.

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DiscoverToolsTool));
}

struct DiscoverToolsTool;

impl Tool for DiscoverToolsTool {
    fn name(&self) -> &'static str {
        "discover_tools"
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Safe
    }

    fn category(&self) -> &'static str {
        "meta"
    }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "discover_tools",
                "description": "Search available tools by keyword or category. Returns a compact list of matching tools with their names and descriptions. Use this when you need a capability not in your current tool set. After discovery, you can call the discovered tools directly in your next response.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search keyword (matches tool names and descriptions)"
                        },
                        "category": {
                            "type": "string",
                            "description": "Filter by category (e.g. 'browser', 'git', 'docker', 'files', 'network')"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str());
        let category = args.get("category").and_then(|v| v.as_str());

        let metadata = match ctx.registry_metadata {
            Some(m) => m,
            None => return "Error: tool metadata not available".to_string(),
        };

        // No filters → category summary
        if query.is_none() && category.is_none() {
            return category_summary(metadata);
        }

        let q_lower = query.map(|q| q.to_lowercase());
        let c_lower = category.map(|c| c.to_lowercase());

        let matched: Vec<_> = metadata
            .iter()
            .filter(|m| {
                let cat_ok = c_lower
                    .as_ref()
                    .map(|c| m.category.to_lowercase().contains(c.as_str()))
                    .unwrap_or(true);
                let query_ok = q_lower
                    .as_ref()
                    .map(|q| {
                        m.name.to_lowercase().contains(q.as_str())
                            || m.description.to_lowercase().contains(q.as_str())
                            || m.category.to_lowercase().contains(q.as_str())
                    })
                    .unwrap_or(true);
                cat_ok && query_ok
            })
            .take(15)
            .collect();

        if matched.is_empty() {
            let cats = list_categories(metadata);
            return format!(
                "No tools found matching query={:?} category={:?}. Available categories: {}",
                query.unwrap_or(""),
                category.unwrap_or(""),
                cats,
            );
        }

        let mut out = format!("Found {} tools:\n", matched.len());
        out.push_str("name | category | permission | description\n");
        out.push_str("---|---|---|---\n");
        for m in &matched {
            out.push_str(&format!(
                "{} | {} | {} | {}\n",
                m.name, m.category, m.permission, m.description
            ));
        }
        out.push_str("\nYou can now call any of these tools directly.");
        out
    }
}

fn category_summary(metadata: &[super::ToolMetadata]) -> String {
    let mut cat_counts: std::collections::BTreeMap<&str, usize> =
        std::collections::BTreeMap::new();
    for m in metadata {
        *cat_counts.entry(m.category).or_insert(0) += 1;
    }

    let mut out = format!(
        "Available tool categories ({} total tools):\n",
        metadata.len()
    );
    for (cat, count) in &cat_counts {
        out.push_str(&format!("- {} ({} tools)\n", cat, count));
    }
    out.push_str("\nUse discover_tools with a query or category to find specific tools.");
    out
}

fn list_categories(metadata: &[super::ToolMetadata]) -> String {
    let mut cats: Vec<&str> = metadata.iter().map(|m| m.category).collect();
    cats.sort();
    cats.dedup();
    cats.join(", ")
}
