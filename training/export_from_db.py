#!/usr/bin/env python3
"""Export training data from PostgreSQL to JSONL for training frameworks."""

import json
import sys
import psycopg2

DB_URL = "postgresql://yantrik:yantrik_train_2026@192.168.4.176:5432/yantrik_training"


def export_dataset(output_path, dataset=None, min_quality=None, bond_stage=None,
                   version_tag=None, limit=None):
    """Export filtered examples to JSONL."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    query = "SELECT conversations, metadata FROM training_examples WHERE 1=1"
    params = []

    if dataset:
        query += " AND dataset = %s"
        params.append(dataset)
    if min_quality is not None:
        query += " AND (quality_score IS NULL OR quality_score >= %s)"
        params.append(min_quality)
    if bond_stage:
        query += " AND bond_stage = %s"
        params.append(bond_stage)

    query += " ORDER BY random()"
    if limit:
        query += f" LIMIT {int(limit)}"

    cur.execute(query, params)
    rows = cur.fetchall()

    with open(output_path, "w") as f:
        for conversations, metadata in rows:
            example = {"conversations": conversations}
            if metadata:
                example["metadata"] = metadata
            f.write(json.dumps(example) + "\n")

    # Record export version
    if version_tag:
        filters = {"dataset": dataset, "min_quality": min_quality,
                   "bond_stage": bond_stage, "limit": limit}
        cur.execute(
            """INSERT INTO dataset_versions (version_tag, description, filters, example_count)
               VALUES (%s, %s, %s, %s)
               ON CONFLICT (version_tag) DO UPDATE SET
                   filters = EXCLUDED.filters, example_count = EXCLUDED.example_count,
                   exported_at = NOW()""",
            (version_tag, f"Export: {dataset or 'all'}", json.dumps(filters), len(rows))
        )
        conn.commit()

    cur.close()
    conn.close()
    print(f"Exported {len(rows)} examples to {output_path}")
    return len(rows)


def stats():
    """Print database statistics."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    cur.execute("SELECT COUNT(*) FROM training_examples")
    total = cur.fetchone()[0]
    print(f"Total examples: {total}\n")

    cur.execute("SELECT dataset, COUNT(*) FROM training_examples GROUP BY dataset ORDER BY COUNT(*) DESC")
    print("By dataset:")
    for row in cur.fetchall():
        print(f"  {row[0]}: {row[1]}")

    cur.execute("SELECT bond_stage, COUNT(*) FROM training_examples WHERE bond_stage IS NOT NULL GROUP BY bond_stage")
    print("\nBy bond stage:")
    for row in cur.fetchall():
        print(f"  {row[0]}: {row[1]}")

    cur.execute("""SELECT family, SUM(example_count) as total
                   FROM tool_coverage GROUP BY family ORDER BY total DESC""")
    print("\nTool coverage by family:")
    for row in cur.fetchall():
        print(f"  {row[0]}: {row[1] or 0}")

    cur.execute("""SELECT tool_name, family FROM tool_coverage
                   WHERE example_count = 0 ORDER BY family, tool_name""")
    uncovered = cur.fetchall()
    if uncovered:
        print(f"\nUncovered tools ({len(uncovered)}):")
        for name, family in uncovered[:20]:
            print(f"  [{family}] {name}")
        if len(uncovered) > 20:
            print(f"  ... and {len(uncovered) - 20} more")

    cur.close()
    conn.close()


if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] == "--stats":
        stats()
    elif sys.argv[1] == "--export":
        # python export_from_db.py --export output.jsonl [--dataset tool_calling] [--min-quality 0.7] [--version v1]
        output = sys.argv[2]
        dataset = None
        min_quality = None
        version_tag = None
        i = 3
        while i < len(sys.argv):
            if sys.argv[i] == "--dataset":
                dataset = sys.argv[i+1]; i += 2
            elif sys.argv[i] == "--min-quality":
                min_quality = float(sys.argv[i+1]); i += 2
            elif sys.argv[i] == "--version":
                version_tag = sys.argv[i+1]; i += 2
            else:
                i += 1
        export_dataset(output, dataset=dataset, min_quality=min_quality, version_tag=version_tag)
