#!/usr/bin/env python3
"""
sync_prompts_to_code.py — Apply refined prompts from prompt_catalog.db back to Rust source files.

Handles three categories:
1. Tool descriptions: Replace "description" values in serde_json::json!() blocks
2. Instinct prompts: Add/update match state.model_tier branches with tier-specific EXECUTE prompts
3. Context.rs system prompts: Add tier-aware branching to bond_instructions() and build_system_prompt()

Usage:
    python scripts/sync_prompts_to_code.py [--dry-run] [--category tool_description|instinct|system]
"""

import argparse
import json
import os
import re
import sqlite3
import sys
from pathlib import Path

DB_PATH = "prompt_catalog.db"
ROOT = Path(__file__).resolve().parent.parent


def get_db():
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    return conn


# ════════════════════════════════════════════════════════════════════
#  1. TOOL DESCRIPTIONS
# ════════════════════════════════════════════════════════════════════

def sync_tool_descriptions(dry_run=False):
    """Replace tool description strings in serde_json::json!() blocks."""
    conn = get_db()
    rows = conn.execute("""
        SELECT name, file_path, current_prompt, proposed_small
        FROM prompts
        WHERE category = 'tool_description'
          AND proposed_small IS NOT NULL
          AND proposed_small != ''
          AND proposed_small != current_prompt
    """).fetchall()

    print(f"\n{'[DRY RUN] ' if dry_run else ''}Syncing {len(rows)} tool descriptions...")

    changed_files = {}
    skipped = 0
    errors = []

    for row in rows:
        file_path = ROOT / row["file_path"]
        if not file_path.exists():
            errors.append(f"  MISSING: {row['file_path']}")
            continue

        # Cache file contents
        key = str(file_path)
        if key not in changed_files:
            changed_files[key] = file_path.read_text(encoding="utf-8")

        content = changed_files[key]
        current = row["current_prompt"]
        proposed = row["proposed_small"]

        # The description in JSON is stored in the catalog potentially truncated
        # by the extraction script. We need to find the actual description line.
        # Strategy: find the "description": "..." line that starts with the current_prompt text.

        # Escape for use in regex (the current prompt is literal text in a Rust string)
        current_escaped = re.escape(current.strip())

        # Match "description": "...current text..." in serde_json::json!()
        # The description can span the line — look for it as a substring
        pattern = re.compile(
            r'("description":\s*")'
            + r'(' + current_escaped[:60] + r'[^"]*' + r')'
            + r'"',
            re.DOTALL
        )

        match = pattern.search(content)
        if not match:
            # Try simpler: find exact current text in a "description" value
            simple_pat = re.compile(
                r'"description":\s*"([^"]*' + re.escape(current.strip()[:40]) + r'[^"]*)"'
            )
            match = simple_pat.search(content)
            if not match:
                skipped += 1
                continue

        old_full = match.group(0)
        # Extract just the description value
        desc_match = re.search(r'"description":\s*"(.*?)"', old_full, re.DOTALL)
        if not desc_match:
            skipped += 1
            continue

        old_desc = desc_match.group(1)
        new_full = old_full.replace(old_desc, proposed.strip())

        if old_full == new_full:
            skipped += 1
            continue

        changed_files[key] = content.replace(old_full, new_full, 1)

    # Write changed files
    written = 0
    for fpath, content in changed_files.items():
        original = Path(fpath).read_text(encoding="utf-8")
        if content != original:
            written += 1
            if not dry_run:
                Path(fpath).write_text(content, encoding="utf-8")
            print(f"  {'Would update' if dry_run else 'Updated'}: {os.path.relpath(fpath, ROOT)}")

    print(f"  Files changed: {written}, Skipped: {skipped}, Errors: {len(errors)}")
    for e in errors[:5]:
        print(e)

    conn.close()
    return written


# ════════════════════════════════════════════════════════════════════
#  2. INSTINCT PROMPTS
# ════════════════════════════════════════════════════════════════════

def sync_instinct_prompts(dry_run=False):
    """Add/update tier-aware match blocks in instinct evaluate() methods."""
    conn = get_db()
    rows = conn.execute("""
        SELECT name, file_path, current_prompt, proposed_small, proposed_tiny,
               proposed_medium, proposed_large
        FROM prompts
        WHERE category = 'instinct'
          AND proposed_small IS NOT NULL
          AND proposed_small != ''
    """).fetchall()

    print(f"\n{'[DRY RUN] ' if dry_run else ''}Syncing {len(rows)} instinct prompts...")

    changed_files = {}
    updated = 0
    skipped = 0
    errors = []

    for row in rows:
        file_path = ROOT / row["file_path"]
        if not file_path.exists():
            errors.append(f"  MISSING: {row['file_path']}")
            continue

        key = str(file_path)
        if key not in changed_files:
            changed_files[key] = file_path.read_text(encoding="utf-8")

        content = changed_files[key]
        current = row["current_prompt"]
        proposed_small = row["proposed_small"].strip()
        proposed_tiny = (row["proposed_tiny"] or "").strip()
        proposed_large = (row["proposed_large"] or "").strip()
        proposed_medium = (row["proposed_medium"] or "").strip()

        # Skip if no meaningful small proposal
        if not proposed_small:
            skipped += 1
            continue

        # Check if this file already has model_tier branching
        has_tier_match = "match state.model_tier" in content or "model_tier" in content

        # Find the EXECUTE format!() block for this instinct
        # Pattern: format!("EXECUTE ..." or format!(\n"EXECUTE ...
        # We need to find the specific EXECUTE block matching current_prompt

        # Strategy: find `format!(\n            "EXECUTE ` blocks
        # Use first 40 chars of current_prompt to locate

        current_start = current.strip()[:50]
        if not current_start:
            skipped += 1
            continue

        # If file already has tier matching, we need to update the existing branches
        if has_tier_match and "match state.model_tier" in content:
            result = update_existing_tier_match(content, row, proposed_small, proposed_tiny, proposed_large, proposed_medium)
            if result and result != content:
                changed_files[key] = result
                updated += 1
            else:
                skipped += 1
            continue

        # Find the format!("EXECUTE ...") block
        # Look for the line containing current_start
        execute_idx = find_execute_block(content, current_start)
        if execute_idx is None:
            # Try alternate: look for any format! containing first 30 chars
            execute_idx = find_execute_block(content, current_start[:30])

        if execute_idx is None:
            skipped += 1
            continue

        # Extract the full format!(...) expression
        fmt_start, fmt_end = extract_format_block(content, execute_idx)
        if fmt_start is None:
            skipped += 1
            continue

        old_block = content[fmt_start:fmt_end]

        # Build tier-aware replacement
        new_block = build_tier_match_block(
            old_block, proposed_small, proposed_tiny, proposed_large, proposed_medium,
            content, fmt_start
        )

        if new_block and new_block != old_block:
            # Ensure ModelTier import exists
            new_content = content[:fmt_start] + new_block + content[fmt_end:]
            new_content = ensure_model_tier_import(new_content)
            changed_files[key] = new_content
            updated += 1
        else:
            skipped += 1

    # Write changed files
    written = 0
    for fpath, content in changed_files.items():
        original = Path(fpath).read_text(encoding="utf-8")
        if content != original:
            written += 1
            if not dry_run:
                Path(fpath).write_text(content, encoding="utf-8")
            print(f"  {'Would update' if dry_run else 'Updated'}: {os.path.relpath(fpath, ROOT)}")

    print(f"  Files changed: {written}, Updated: {updated}, Skipped: {skipped}, Errors: {len(errors)}")
    for e in errors[:5]:
        print(e)

    conn.close()
    return written


def find_execute_block(content: str, needle: str) -> int | None:
    """Find the index where an EXECUTE format block starts containing needle."""
    # Search for the needle text (escaped for Rust string)
    idx = content.find(needle)
    if idx == -1:
        # Try without EXECUTE prefix
        clean = needle.replace("EXECUTE ", "")
        idx = content.find(clean[:30])
    if idx == -1:
        return None
    return idx


def extract_format_block(content: str, ref_idx: int) -> tuple[int | None, int | None]:
    """Extract the full format!(...) expression containing ref_idx.

    Returns (start, end) of the complete `let execute_msg = format!(...);` or
    the `format!("EXECUTE ...")` expression.
    """
    # Walk backwards from ref_idx to find `format!(` or `let execute_msg = format!(`
    search_start = max(0, ref_idx - 500)
    segment = content[search_start:ref_idx]

    # Find the last `format!(` before ref_idx
    fmt_positions = [m.start() + search_start for m in re.finditer(r'format!\s*\(', segment)]
    if not fmt_positions:
        return None, None

    fmt_start = fmt_positions[-1]

    # Now find the variable assignment start (let execute_msg = ...)
    # Walk backwards from fmt_start to find `let ... = `
    let_search = content[max(0, fmt_start - 200):fmt_start]
    let_match = re.search(r'(let\s+\w+\s*=\s*)$', let_search)
    if let_match:
        actual_start = fmt_start - len(let_search) + let_match.start()
    else:
        actual_start = fmt_start

    # Find matching closing paren — count parens
    depth = 0
    i = content.index('(', fmt_start)
    for j in range(i, len(content)):
        c = content[j]
        if c == '(':
            depth += 1
        elif c == ')':
            depth -= 1
            if depth == 0:
                # Include trailing semicolon if present
                end = j + 1
                if end < len(content) and content[end] == ';':
                    end += 1
                return actual_start, end

    return None, None


def build_tier_match_block(
    old_block: str,
    proposed_small: str,
    proposed_tiny: str,
    proposed_large: str,
    proposed_medium: str,
    full_content: str,
    block_start: int,
) -> str | None:
    """Build a match state.model_tier { ... } block replacing a single format!() call.

    Strategy:
    - Large branch: Keep original format!() exactly as-is (correct vars in scope)
    - Small/Tiny: Use proposed templates, mapping template vars to available Rust vars
    """

    # Detect indentation from old_block position
    line_start = full_content.rfind('\n', 0, block_start) + 1
    indent = ""
    for ch in full_content[line_start:block_start]:
        if ch in (' ', '\t'):
            indent += ch
        else:
            break

    # Determine which variable the old block assigned to
    var_match = re.match(r'let\s+(\w+)\s*=\s*', old_block)
    var_name = var_match.group(1) if var_match else "execute_msg"

    # Find what Rust format variables exist in the original format!() call
    original_vars = set(re.findall(r'\{(\w+)\}', old_block))

    def ensure_execute_prefix(text: str) -> str:
        t = text.strip()
        return t if t.startswith("EXECUTE ") else "EXECUTE " + t

    def map_template_vars(template: str, available_vars: set) -> str:
        """Map template placeholder vars to available Rust variables.

        Template vars like {user}, {interest}, {tone} must be mapped to
        actual Rust variables in scope. Unknown vars are replaced with
        literal text to avoid compile errors.
        """
        # Mapping from template placeholders to likely Rust variable names
        var_aliases = {
            'user': ['user'],
            'interest': ['interests_str', 'interest'],
            'tone': [],  # No standard var — will be removed
            'time': ['time_str'],  # Doesn't usually exist, remove
            'topic': ['topic', 'topic_str'],
            'event': ['event', 'event_str'],
            'context': ['context_str', 'context'],
            'category': ['category', 'cat'],
            'location': ['location'],
            'name': ['name'],
        }

        result = template
        for var in re.findall(r'\{(\w+)\}', template):
            if var in available_vars:
                continue  # Already a valid Rust variable

            # Try aliases
            mapped = False
            for alias in var_aliases.get(var, []):
                if alias in available_vars:
                    result = result.replace('{' + var + '}', '{' + alias + '}')
                    mapped = True
                    break

            if not mapped:
                # Variable doesn't exist in scope — escape the braces so it's literal text
                # e.g., {tone} -> {{tone}} which Rust format!() renders as literal "{tone}"
                # But that looks weird, so just remove the placeholder entirely
                if var == 'tone':
                    result = result.replace('Tone: {tone}.', '')
                    result = result.replace('Tone: {tone}', '')
                    result = result.replace('{tone}', '')
                elif var == 'time':
                    result = result.replace('{time}', '(now)')
                else:
                    result = result.replace('{' + var + '}', '')

        # Clean up double spaces and trailing dots
        result = re.sub(r'  +', ' ', result)
        result = result.strip()
        return result

    # Extract original format!() content for the Large branch
    fmt_match = re.search(r'format!\s*\(([\s\S]*)\)\s*;?\s*$', old_block)
    if not fmt_match:
        return None
    fmt_body = fmt_match.group(1).strip()

    # Build the match block
    lines = []
    lines.append(f"let {var_name} = match state.model_tier {{")

    # Large tier — keep original format!() exactly
    lines.append(f'{indent}    ModelTier::Large => format!(')
    lines.append(f'{indent}        {fmt_body}')
    lines.append(f'{indent}    ),')

    # Small tier (default fallback)
    small_text = ensure_execute_prefix(map_template_vars(proposed_small, original_vars))
    small_escaped = escape_for_rust_string(small_text)

    # Tiny tier
    if proposed_tiny:
        tiny_text = ensure_execute_prefix(map_template_vars(proposed_tiny, original_vars))
        tiny_escaped = escape_for_rust_string(tiny_text)
        lines.append(f'{indent}    ModelTier::Tiny => format!(')
        lines.append(f'{indent}        "{tiny_escaped}",')
        lines.append(f'{indent}    ),')
        lines.append(f'{indent}    _ => format!(')
        lines.append(f'{indent}        "{small_escaped}",')
        lines.append(f'{indent}    ),')
    else:
        lines.append(f'{indent}    _ => format!(')
        lines.append(f'{indent}        "{small_escaped}",')
        lines.append(f'{indent}    ),')

    lines.append(f"{indent}}};")

    return indent + "\n".join(lines)


def escape_for_rust_string(text: str) -> str:
    """Escape text for use inside a Rust format!() string literal."""
    # The text contains {user}, {time} etc — these are format args
    # We need to preserve {} pairs but escape lone braces
    text = text.replace('\\', '\\\\')
    text = text.replace('"', '\\"')
    text = text.replace('\n', '\\n\\\n             ')
    return text


def ensure_model_tier_import(content: str) -> str:
    """Ensure ModelTier is imported in the file."""
    if "ModelTier" in content:
        # Check if it's in an import
        if re.search(r'use\s+.*ModelTier', content):
            return content
        # It's referenced but not imported — might be a match arm
        # Check the existing import line
        import_match = re.search(
            r'(use yantrik_companion_core::types::\{[^}]*)\}',
            content
        )
        if import_match:
            imports = import_match.group(1)
            if "ModelTier" not in imports:
                new_imports = imports + ", ModelTier}"
                return content.replace(import_match.group(0), new_imports)
        return content

    # Add ModelTier to existing import
    import_match = re.search(
        r'(use yantrik_companion_core::types::\{)([^}]*)\}',
        content
    )
    if import_match:
        prefix = import_match.group(1)
        existing = import_match.group(2).strip()
        new_import = f"{prefix}{existing}, ModelTier}}"
        return content.replace(import_match.group(0), new_import)

    # No existing import — add one after the last `use` line
    last_use = 0
    for m in re.finditer(r'^use .*;\n', content, re.MULTILINE):
        last_use = m.end()
    if last_use:
        return content[:last_use] + "use yantrik_companion_core::types::ModelTier;\n" + content[last_use:]

    return content


def update_existing_tier_match(content, row, proposed_small, proposed_tiny, proposed_large, proposed_medium):
    """Update an existing match state.model_tier block with new proposals."""
    # For files that already have tier matching, we update the specific branch contents
    # This is complex — for now, just skip if already has tier matching
    # The 7 files with existing tier matching were already manually tuned
    return None


# ════════════════════════════════════════════════════════════════════
#  3. CONTEXT.RS SYSTEM PROMPTS
# ════════════════════════════════════════════════════════════════════

def sync_context_prompts(dry_run=False):
    """Add tier-aware branching to context.rs system prompt functions."""
    conn = get_db()

    file_path = ROOT / "crates" / "yantrik-companion" / "src" / "context.rs"
    if not file_path.exists():
        print(f"  ERROR: context.rs not found")
        return 0

    content = file_path.read_text(encoding="utf-8")
    original = content

    # ── Bond instructions ──
    bond_rows = conn.execute("""
        SELECT name, proposed_small
        FROM prompts
        WHERE name LIKE 'bond_instructions_%'
          AND proposed_small IS NOT NULL
          AND proposed_small != ''
    """).fetchall()

    if bond_rows:
        content = update_bond_instructions(content, {r["name"]: r["proposed_small"] for r in bond_rows})

    # ── Main system prompt — add tier-aware build_system_prompt ──
    main_row = conn.execute("""
        SELECT proposed_small, proposed_tiny
        FROM prompts
        WHERE name = 'main_system_prompt'
    """).fetchone()

    if main_row and main_row["proposed_small"]:
        content = add_tier_aware_system_prompt(content, main_row["proposed_small"])

    if content != original:
        print(f"  {'Would update' if dry_run else 'Updated'}: crates/yantrik-companion/src/context.rs")
        if not dry_run:
            file_path.write_text(content, encoding="utf-8")
        return 1

    print("  No changes to context.rs")
    conn.close()
    return 0


def update_bond_instructions(content: str, bond_map: dict) -> str:
    """Update bond_instructions() to include tier-aware 2-word tags for Small tier."""

    old_sig = "fn bond_instructions(level: BondLevel, name: &str, user: &str) -> String {"
    if old_sig not in content:
        print("  WARN: bond_instructions signature not found, skipping")
        return content

    if "model_tier: &ModelTier" in content:
        return content  # Already updated

    tag_map = {
        "stranger": bond_map.get("bond_instructions_stranger", "polite, helpful"),
        "acquaintance": bond_map.get("bond_instructions_acquaintance", "warm, measured"),
        "friend": bond_map.get("bond_instructions_friend", "casual, direct"),
        "confidant": bond_map.get("bond_instructions_confidant", "warm, attentive"),
        "partner": bond_map.get("bond_instructions_partner", "warm, candid"),
    }

    # Find the full old function body (matching braces)
    func_start = content.find(old_sig)
    depth = 0
    func_body_start = content.index('{', func_start)
    func_end = func_body_start
    for i in range(func_body_start, len(content)):
        if content[i] == '{':
            depth += 1
        elif content[i] == '}':
            depth -= 1
            if depth == 0:
                func_end = i + 1
                break

    # Read the old function body to preserve Medium/Large text exactly
    old_func = content[func_start:func_end]

    # Extract each BondLevel match arm's format!() content from the original
    # We'll keep these exactly for Medium/Large
    old_arms = {}
    for level_name, enum_name in [
        ("stranger", "BondLevel::Stranger"),
        ("acquaintance", "BondLevel::Acquaintance"),
        ("friend", "BondLevel::Friend"),
        ("confidant", "BondLevel::Confidant"),
        ("partner", "BondLevel::PartnerInCrime"),
    ]:
        arm_match = re.search(
            rf'{re.escape(enum_name)}\s*=>\s*format!\(\s*([\s\S]*?)\s*\),',
            old_func
        )
        if arm_match:
            old_arms[level_name] = arm_match.group(1).strip()

    # Build new function
    new_func = f"""fn bond_instructions(level: BondLevel, name: &str, user: &str, model_tier: &ModelTier) -> String {{
    match model_tier {{
        ModelTier::Tiny | ModelTier::Small => {{
            // 2-word bond tone tags for small models
            let tag = match level {{
                BondLevel::Stranger => "{tag_map['stranger']}",
                BondLevel::Acquaintance => "{tag_map['acquaintance']}",
                BondLevel::Friend => "{tag_map['friend']}",
                BondLevel::Confidant => "{tag_map['confidant']}",
                BondLevel::PartnerInCrime => "{tag_map['partner']}",
            }};
            tag.to_string()
        }}
        _ => {{
            // Full behavioral descriptions for Medium/Large models
            match level {{
                BondLevel::Stranger => format!(
                    {old_arms.get('stranger', '"(unknown)"')}
                ),
                BondLevel::Acquaintance => format!(
                    {old_arms.get('acquaintance', '"(unknown)"')}
                ),
                BondLevel::Friend => format!(
                    {old_arms.get('friend', '"(unknown)"')}
                ),
                BondLevel::Confidant => format!(
                    {old_arms.get('confidant', '"(unknown)"')}
                ),
                BondLevel::PartnerInCrime => format!(
                    {old_arms.get('partner', '"(unknown)"')}
                ),
            }}
        }}
    }}
}}"""

    content = content[:func_start] + new_func + content[func_end:]

    # Update call site
    content = content.replace(
        "bond_instructions(level, name, user)",
        "bond_instructions(level, name, user, &state.model_tier)"
    )

    # Ensure ModelTier import — add to existing crate::types import
    if not re.search(r'use\s+.*ModelTier', content):
        old_import = "use crate::types::{CompanionState, Urge};"
        if old_import in content:
            content = content.replace(
                old_import,
                "use crate::types::{CompanionState, ModelTier, Urge};"
            )
        else:
            content = content.replace(
                "use crate::bond::BondLevel;",
                "use crate::bond::BondLevel;\nuse crate::types::ModelTier;"
            )

    return content


def add_tier_aware_system_prompt(content: str, proposed_small: str) -> str:
    """Add Small-tier system prompt as an early return in build_system_prompt().

    For Small/Tiny models, we skip the section-by-section assembly and return
    a compact, pre-structured prompt template.
    """

    # Insert after the `let level = state.bond_level;` line
    anchor = "let level = state.bond_level;"
    anchor_idx = content.find(anchor)
    if anchor_idx == -1:
        print("  WARN: could not find anchor for tier-aware system prompt")
        return content

    # Check if already inserted
    if "Small/Tiny: compact template" in content:
        return content

    insert_point = content.index('\n', anchor_idx) + 1

    # Build the compact template insertion
    # The proposed_small has placeholders: {name}, {user}, {time}, {bond_tag}, {top_3_memories}, {urge_or_none}
    # We need to fill these from the available variables in scope

    compact_block = r'''
    // ── Small/Tiny: compact template (replaces section-by-section assembly) ──
    if matches!(state.model_tier, ModelTier::Tiny | ModelTier::Small) {
        let now = chrono::Local::now();
        let time_str = now.format("%a %b %d %I:%M%p").to_string();
        let bond_tag = bond_instructions(level, name, user, &state.model_tier);

        let top_memories = if memories.is_empty() {
            "none".to_string()
        } else {
            memories.iter().take(3)
                .map(|m| {
                    let t = if m.text.len() > 100 {
                        &m.text[..m.text.char_indices()
                            .take_while(|&(i, _)| i < 100)
                            .last()
                            .map(|(i, c)| i + c.len_utf8())
                            .unwrap_or(100)]
                    } else {
                        &m.text
                    };
                    sanitize::escape_for_prompt(t)
                })
                .collect::<Vec<_>>()
                .join("; ")
        };

        let urge_hint = if urges.is_empty() {
            "none".to_string()
        } else {
            sanitize::escape_for_prompt(&urges[0].reason)
        };

        let mut prompt = String::with_capacity(600);
        prompt.push_str("/no_think\n");
        prompt.push_str(&format!(
            "You are {name}, {user}'s companion.\n\
             \n\
             Rules:\n\
             1. Use tools for current, external, or uncertain facts. If no tool is needed, answer directly. Never present guesses as facts.\n\
             2. Act immediately when the task is clear. Ask a brief question if a missing detail would change the result or action.\n\
             3. If full completion is blocked, do any safe partial progress and clearly say what remains.\n\
             4. Be concise. Match tone without changing facts.\n\
             \n\
             State:\n\
             - Time: {time_str}\n\
             - Tone: {bond_tag}\n\
             - Preferences (may be stale): {top_memories}\n\
             - Hint: {urge_hint}\n"
        ));

        // Add tool chaining for Small (not Tiny)
        if matches!(state.model_tier, ModelTier::Small) && config.tools.enabled {
            prompt.push_str(
                "\nTool rules: Call tools immediately. Never narrate actions. After tools, give a short natural reply.\n"
            );
        }

        prompt.push_str(&security_instructions());
        return prompt;
    }

'''

    content = content[:insert_point] + compact_block + content[insert_point:]
    return content


# ════════════════════════════════════════════════════════════════════
#  MAIN
# ════════════════════════════════════════════════════════════════════

def main():
    parser = argparse.ArgumentParser(description="Sync refined prompts to Rust source")
    parser.add_argument("--dry-run", action="store_true", help="Show changes without writing")
    parser.add_argument("--category", choices=["tool_description", "instinct", "system", "all"],
                        default="all", help="Which category to sync")
    args = parser.parse_args()

    total = 0

    if args.category in ("tool_description", "all"):
        total += sync_tool_descriptions(args.dry_run)

    if args.category in ("instinct", "all"):
        total += sync_instinct_prompts(args.dry_run)

    if args.category in ("system", "all"):
        total += sync_context_prompts(args.dry_run)

    print(f"\n{'[DRY RUN] ' if args.dry_run else ''}Total files modified: {total}")


if __name__ == "__main__":
    main()
