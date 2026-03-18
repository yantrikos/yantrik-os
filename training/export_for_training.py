#!/usr/bin/env python3
"""Export training data from PostgreSQL into unsloth/SFT training format.

Outputs:
  training/output/train_sft.jsonl  — SFT training data (ShareGPT format)
  training/output/eval_sft.jsonl   — SFT eval data
  training/output/train_dpo.jsonl  — DPO training data
"""

import json
import os
import random
import sys
from collections import defaultdict

import psycopg2

DB_URL = "postgresql://yantrik:yantrik_train_2026@192.168.4.176:5432/yantrik_training"
OUTPUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "output")
EVAL_FRACTION = 0.05
SEED = 42


def fetch_all_examples(only_untrained=False, model_size=None):
    """Fetch training examples from the database.

    Args:
        only_untrained: Only fetch examples not yet trained for the given model.
        model_size: Model size to check training status against (e.g. '4b', '9b').
                    Required when only_untrained=True.
    """
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()
    query = (
        "SELECT te.id, te.dataset, te.conversations, te.metadata, "
        "te.tools_used, te.bond_stage, te.scenario_type "
        "FROM training_examples te"
    )
    params = []
    if only_untrained and model_size:
        query += (
            " WHERE NOT EXISTS ("
            "   SELECT 1 FROM training_runs tr"
            "   WHERE tr.example_id = te.id AND tr.model_size = %s"
            " )"
        )
        params.append(model_size)
    query += " ORDER BY te.id"
    cur.execute(query, params)
    rows = cur.fetchall()
    cur.close()
    conn.close()
    return rows


def parse_json_field(value):
    """Parse a JSON field that may be a string or already a dict/list."""
    if value is None:
        return None
    if isinstance(value, (dict, list)):
        return value
    return json.loads(value)


def extract_conversations(raw):
    """Extract the message list from the conversations field.

    Handles:
      - A JSON array of messages directly
      - A JSON object with a "conversations" or "messages" key
    """
    parsed = parse_json_field(raw)
    if parsed is None:
        return None
    if isinstance(parsed, list):
        return parsed
    if isinstance(parsed, dict):
        if "conversations" in parsed:
            return parsed["conversations"]
        if "messages" in parsed:
            return parsed["messages"]
    return None


def map_role(role):
    """Map OpenAI-style roles to ShareGPT/unsloth roles."""
    mapping = {
        "system": "system",
        "user": "human",
        "assistant": "gpt",
        "tool": "tool",
    }
    return mapping.get(role, role)


def has_tool_calls(messages):
    """Check if any message in the conversation has tool_calls."""
    return any(
        msg.get("tool_calls") and msg.get("role") == "assistant"
        for msg in messages
    )


def normalize_tool_call(tc):
    """Ensure a tool_call dict has proper OpenAI format."""
    if not isinstance(tc, dict):
        return tc
    func = tc.get("function", {})
    # Parse stringified arguments back to dict
    args = func.get("arguments", "{}")
    if isinstance(args, str):
        try:
            args = json.loads(args)
        except json.JSONDecodeError:
            pass
    return {
        "id": tc.get("id", "call_0"),
        "type": "function",
        "function": {
            "name": func.get("name", ""),
            "arguments": args,
        },
    }


def convert_to_openai(messages):
    """Keep messages in OpenAI format, normalizing tool_calls.

    Used for tool-calling examples so apply_chat_template(tools=...)
    can properly format them with Qwen3.5's native tool calling Jinja template.
    """
    result = []
    for msg in messages:
        role = msg.get("role", "")
        content = msg.get("content")
        tool_calls = msg.get("tool_calls")

        if role == "assistant" and tool_calls:
            normalized_tcs = [normalize_tool_call(tc) for tc in tool_calls]
            result.append({
                "role": "assistant",
                "content": content or "",
                "tool_calls": normalized_tcs,
            })
        elif role == "tool":
            result.append({
                "role": "tool",
                "tool_call_id": msg.get("tool_call_id", "call_0"),
                "name": msg.get("name", ""),
                "content": content or "",
            })
        else:
            result.append({"role": role, "content": content or ""})

    return result


def convert_to_sharegpt(messages):
    """Convert a list of role/content messages to ShareGPT from/value format.

    Used for non-tool-calling examples (pure conversation).
    Tool-calling examples use convert_to_openai() instead.
    """
    result = []
    for msg in messages:
        role = msg.get("role", "")
        content = msg.get("content")
        sharegpt_role = map_role(role)
        result.append({"from": sharegpt_role, "value": content or ""})

    return result


def is_dpo_example(raw_conversations, metadata):
    """Check if this example is a DPO example (has chosen/rejected)."""
    parsed = parse_json_field(raw_conversations)
    if isinstance(parsed, dict):
        if "chosen" in parsed or "rejected" in parsed:
            return True
    meta = parse_json_field(metadata)
    if isinstance(meta, dict):
        if "chosen" in meta or "rejected" in meta:
            return True
    # Also check the raw data at top level
    return False


def extract_dpo(raw_conversations, metadata):
    """Extract DPO fields from an example.

    Returns (prompt_messages, chosen_text, rejected_text) or None.
    """
    parsed = parse_json_field(raw_conversations)
    meta = parse_json_field(metadata) or {}

    conversations = None
    chosen = None
    rejected = None

    if isinstance(parsed, dict):
        conversations = parsed.get("conversations") or parsed.get("messages")
        chosen = parsed.get("chosen")
        rejected = parsed.get("rejected")
    elif isinstance(parsed, list):
        conversations = parsed

    # Also check metadata
    if chosen is None and "chosen" in meta:
        chosen = meta["chosen"]
    if rejected is None and "rejected" in meta:
        rejected = meta["rejected"]

    if chosen is None or rejected is None:
        return None

    # Extract chosen/rejected text
    chosen_text = chosen.get("content", "") if isinstance(chosen, dict) else str(chosen)
    rejected_text = rejected.get("content", "") if isinstance(rejected, dict) else str(rejected)

    # Build prompt as ShareGPT turns (system + user messages only)
    prompt_turns = []
    if conversations:
        for msg in conversations:
            role = msg.get("role", "")
            if role in ("system", "user"):
                prompt_turns.append({
                    "from": map_role(role),
                    "value": msg.get("content", ""),
                })

    return prompt_turns, chosen_text, rejected_text


def stratified_split(examples_by_dataset, eval_fraction, seed):
    """Split examples into train/eval, stratified by dataset category.

    Returns (train_list, eval_list).
    """
    rng = random.Random(seed)
    train = []
    eval_ = []

    for dataset, examples in examples_by_dataset.items():
        shuffled = list(examples)
        rng.shuffle(shuffled)
        n_eval = max(1, int(len(shuffled) * eval_fraction))
        if len(shuffled) <= 2:
            # Too few — put all in train
            train.extend(shuffled)
        else:
            eval_.extend(shuffled[:n_eval])
            train.extend(shuffled[n_eval:])

    # Shuffle the final lists
    rng.shuffle(train)
    rng.shuffle(eval_)
    return train, eval_


def main():
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--untrained-only", action="store_true",
                        help="Only export examples not yet trained for the given model")
    parser.add_argument("--model", type=str, default=None,
                        help="Model size to check training status (e.g. '4b', '9b')")
    args = parser.parse_args()

    if args.untrained_only and not args.model:
        print("Error: --model is required with --untrained-only")
        sys.exit(1)

    os.makedirs(OUTPUT_DIR, exist_ok=True)

    mode = f"untrained for {args.model}" if args.untrained_only else "all"
    print(f"Fetching examples from database ({mode})...")
    rows = fetch_all_examples(only_untrained=args.untrained_only, model_size=args.model)
    print(f"Fetched {len(rows)} total rows\n")

    sft_by_dataset = defaultdict(list)
    dpo_examples = []
    skipped = 0
    dataset_counts = defaultdict(int)

    for row_id, dataset, conversations_raw, metadata_raw, tools_used, bond_stage, scenario_type in rows:
        dataset_key = dataset or "unknown"
        dataset_counts[dataset_key] += 1

        # Check if DPO
        if is_dpo_example(conversations_raw, metadata_raw):
            result = extract_dpo(conversations_raw, metadata_raw)
            if result:
                prompt_turns, chosen_text, rejected_text = result
                dpo_examples.append({
                    "prompt": prompt_turns,
                    "chosen": chosen_text,
                    "rejected": rejected_text,
                })
            continue

        # SFT example
        messages = extract_conversations(conversations_raw)
        if not messages:
            skipped += 1
            continue

        if has_tool_calls(messages):
            # Tool-calling example: keep OpenAI format for native tool calling training
            openai_msgs = convert_to_openai(messages)
            if not openai_msgs:
                skipped += 1
                continue
            # Extract tool names used to attach relevant schemas at training time
            tool_names = set()
            for m in openai_msgs:
                for tc in m.get("tool_calls", []):
                    name = tc.get("function", {}).get("name", "")
                    if name:
                        tool_names.add(name)
            sft_by_dataset[dataset_key].append({
                "messages": openai_msgs,
                "tools_used": sorted(tool_names),
                "format": "openai",
            })
        else:
            # Non-tool example: ShareGPT format
            sharegpt = convert_to_sharegpt(messages)
            if not sharegpt:
                skipped += 1
                continue
            sft_by_dataset[dataset_key].append({
                "conversations": sharegpt,
                "format": "sharegpt",
            })

    # Split SFT into train/eval
    train_sft, eval_sft = stratified_split(sft_by_dataset, EVAL_FRACTION, SEED)

    # Write SFT files
    train_path = os.path.join(OUTPUT_DIR, "train_sft.jsonl")
    eval_path = os.path.join(OUTPUT_DIR, "eval_sft.jsonl")
    dpo_path = os.path.join(OUTPUT_DIR, "train_dpo.jsonl")

    with open(train_path, "w", encoding="utf-8") as f:
        for ex in train_sft:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    with open(eval_path, "w", encoding="utf-8") as f:
        for ex in eval_sft:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    with open(dpo_path, "w", encoding="utf-8") as f:
        for ex in dpo_examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")

    # Print stats
    total_sft = len(train_sft) + len(eval_sft)
    print("=" * 60)
    print("EXPORT COMPLETE")
    print("=" * 60)
    print(f"Total rows fetched:    {len(rows)}")
    print(f"SFT examples:          {total_sft} (train: {len(train_sft)}, eval: {len(eval_sft)})")
    print(f"DPO examples:          {len(dpo_examples)}")
    print(f"Skipped (no convos):   {skipped}")
    print()
    print("Examples per dataset:")
    for ds in sorted(dataset_counts.keys()):
        print(f"  {ds:30s}  {dataset_counts[ds]}")
    print()
    print(f"Output files:")
    print(f"  {train_path}")
    print(f"  {eval_path}")
    print(f"  {dpo_path}")


if __name__ == "__main__":
    main()
