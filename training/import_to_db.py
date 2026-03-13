#!/usr/bin/env python3
"""Import JSONL training data files into PostgreSQL."""

import json
import hashlib
import glob
import sys
import psycopg2

DB_URL = "postgresql://yantrik:yantrik_train_2026@192.168.4.176:5432/yantrik_training"


def import_jsonl(filepath, dataset, batch):
    """Import a JSONL file into training_examples."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    imported = 0
    skipped = 0

    with open(filepath, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                data = json.loads(line)
            except json.JSONDecodeError:
                skipped += 1
                continue

            conversations = data.get("conversations", data)
            metadata = data.get("metadata", {})

            # Preserve DPO chosen/rejected in metadata
            if "chosen" in data:
                metadata["chosen"] = data["chosen"]
            if "rejected" in data:
                metadata["rejected"] = data["rejected"]
            content_hash = hashlib.md5(json.dumps(conversations, sort_keys=True).encode()).hexdigest()

            # Extract tools used from conversations
            tools_used = []
            for msg in conversations if isinstance(conversations, list) else []:
                if msg.get("role") == "assistant" and msg.get("tool_calls"):
                    for tc in msg["tool_calls"]:
                        name = tc.get("name", tc.get("function", {}).get("name", ""))
                        if name:
                            tools_used.append(name)
                if msg.get("role") == "tool" and msg.get("name"):
                    tools_used.append(msg["name"])
            tools_used = list(set(tools_used))

            bond_stage = metadata.get("bond_stage")
            scenario_type = metadata.get("scenario_type")

            try:
                cur.execute(
                    """INSERT INTO training_examples
                       (dataset, batch, conversations, metadata, tools_used, bond_stage, scenario_type, content_hash)
                       VALUES (%s, %s, %s, %s, %s, %s, %s, %s)
                       ON CONFLICT (content_hash) DO NOTHING""",
                    (dataset, batch, json.dumps(conversations), json.dumps(metadata),
                     tools_used or None, bond_stage, scenario_type, content_hash)
                )
                if cur.rowcount > 0:
                    imported += 1
                else:
                    skipped += 1
            except Exception as e:
                print(f"  Error: {e}")
                conn.rollback()
                skipped += 1
                continue

    conn.commit()

    # Update tool coverage counts
    if tools_used:
        cur.execute("""
            UPDATE tool_coverage SET
                example_count = (SELECT COUNT(*) FROM training_examples WHERE %s = ANY(tools_used)),
                last_updated = NOW()
            WHERE tool_name = ANY(%s)
        """, (tools_used[0], tools_used))

    conn.commit()
    cur.close()
    conn.close()
    return imported, skipped


def import_all():
    """Import all JSONL files from training/data/."""
    files = glob.glob("c:/Users/sync/codes/yantrik-os/training/data/*.jsonl")
    if not files:
        print("No JSONL files found in training/data/")
        return

    total_imported = 0
    total_skipped = 0

    for filepath in sorted(files):
        filename = filepath.replace("\\", "/").split("/")[-1].replace(".jsonl", "")

        # Derive dataset and batch from filename
        if "communicate" in filename or "schedule" in filename or "remember" in filename or \
           "browse" in filename or "files" in filename or "system" in filename or \
           "delegate" in filename or "world" in filename or "security" in filename or \
           "dev" in filename or "smart_home" in filename or "productivity" in filename or \
           "agent" in filename or "comms" in filename or "vision" in filename or \
           "cross_family" in filename:
            dataset = "tool_calling"
        elif "dpo" in filename:
            dataset = "dpo"
        elif "bond" in filename:
            dataset = "bond"
        elif "degrade" in filename:
            dataset = "degradation"
        elif "silence" in filename or "proactiv" in filename:
            dataset = "silence"
        elif "episode" in filename or "day_in" in filename:
            dataset = "episodes"
        else:
            dataset = "other"

        batch = filename
        print(f"Importing {filename} -> dataset={dataset}, batch={batch}")
        imported, skipped = import_jsonl(filepath, dataset, batch)
        print(f"  Imported: {imported}, Skipped: {skipped}")
        total_imported += imported
        total_skipped += skipped

    print(f"\nTotal: {total_imported} imported, {total_skipped} skipped")

    # Print stats
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()
    cur.execute("SELECT dataset, COUNT(*) FROM training_examples GROUP BY dataset ORDER BY dataset")
    print("\nDataset breakdown:")
    for row in cur.fetchall():
        print(f"  {row[0]}: {row[1]}")
    cur.execute("SELECT COUNT(*) FROM training_examples")
    print(f"\nTotal examples in DB: {cur.fetchone()[0]}")
    cur.close()
    conn.close()


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--file":
        # Import specific file: python import_to_db.py --file path.jsonl dataset batch
        filepath, dataset, batch = sys.argv[2], sys.argv[3], sys.argv[4]
        imported, skipped = import_jsonl(filepath, dataset, batch)
        print(f"Imported: {imported}, Skipped: {skipped}")
    else:
        import_all()
