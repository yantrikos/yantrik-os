#!/usr/bin/env python3
"""Mark training examples as trained and manage per-model training tracking.

Tracks training status per model size (4b, 9b, etc.) so the same data
can be independently tracked across different fine-tunes.

Usage:
  python mark_trained.py --migrate                    # Add training_runs table (run once)
  python mark_trained.py --mark-all 4b v1             # Mark all data as trained for 4b model v1
  python mark_trained.py --mark-new 9b v1             # Mark untrained-for-9b data as trained
  python mark_trained.py --stats                      # Show per-model trained/untrained counts
  python mark_trained.py --stats 4b                   # Show stats for specific model
"""

import sys
import psycopg2

DB_URL = "postgresql://yantrik:yantrik_train_2026@192.168.4.176:5432/yantrik_training"


def migrate():
    """Create training_runs tracking table for per-model training status."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    # Per-model training tracking table
    # Each row = "example X was trained into model Y at version Z"
    cur.execute("""
        CREATE TABLE IF NOT EXISTS training_runs (
            id SERIAL PRIMARY KEY,
            example_id INTEGER NOT NULL REFERENCES training_examples(id) ON DELETE CASCADE,
            model_size VARCHAR NOT NULL,      -- '4b', '9b', '27b', etc.
            version VARCHAR NOT NULL,         -- 'v1', 'v2', etc.
            trained_at TIMESTAMP DEFAULT NOW(),
            UNIQUE (example_id, model_size, version)
        );
    """)

    cur.execute("""
        CREATE INDEX IF NOT EXISTS idx_training_runs_model
        ON training_runs (model_size, version);
    """)

    cur.execute("""
        CREATE INDEX IF NOT EXISTS idx_training_runs_example
        ON training_runs (example_id);
    """)

    # Keep the legacy columns for backward compat but they're no longer primary
    cur.execute("""
        DO $$
        BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_name = 'training_examples' AND column_name = 'trained_at'
            ) THEN
                ALTER TABLE training_examples ADD COLUMN trained_at TIMESTAMP;
            END IF;
            IF NOT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_name = 'training_examples' AND column_name = 'training_version'
            ) THEN
                ALTER TABLE training_examples ADD COLUMN training_version VARCHAR;
            END IF;
        END $$;
    """)

    # Migrate legacy trained_at data into training_runs if any exists
    cur.execute("""
        INSERT INTO training_runs (example_id, model_size, version, trained_at)
        SELECT id, '4b', training_version, trained_at
        FROM training_examples
        WHERE trained_at IS NOT NULL AND training_version IS NOT NULL
        ON CONFLICT (example_id, model_size, version) DO NOTHING
    """)
    migrated = cur.rowcount

    conn.commit()
    cur.close()
    conn.close()
    print(f"Migration complete: created training_runs table")
    if migrated > 0:
        print(f"  Migrated {migrated} legacy records into training_runs")


def mark_all(model_size, version):
    """Mark ALL examples as trained for a specific model+version."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    cur.execute("""
        INSERT INTO training_runs (example_id, model_size, version)
        SELECT id, %s, %s FROM training_examples
        ON CONFLICT (example_id, model_size, version) DO NOTHING
    """, (model_size, version))
    count = cur.rowcount
    conn.commit()
    cur.close()
    conn.close()
    print(f"Marked {count} examples as trained for {model_size} {version}")


def mark_new(model_size, version):
    """Mark examples that haven't been trained for this model as trained."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    cur.execute("""
        INSERT INTO training_runs (example_id, model_size, version)
        SELECT te.id, %s, %s
        FROM training_examples te
        WHERE NOT EXISTS (
            SELECT 1 FROM training_runs tr
            WHERE tr.example_id = te.id AND tr.model_size = %s
        )
        ON CONFLICT (example_id, model_size, version) DO NOTHING
    """, (model_size, version, model_size))
    count = cur.rowcount
    conn.commit()
    cur.close()
    conn.close()
    print(f"Marked {count} new examples as trained for {model_size} {version}")


def stats(model_filter=None):
    """Show per-model trained/untrained breakdown."""
    conn = psycopg2.connect(DB_URL)
    cur = conn.cursor()

    cur.execute("SELECT COUNT(*) FROM training_examples")
    total = cur.fetchone()[0]
    print(f"Total examples in DB: {total}\n")

    # Get all model sizes that have training runs
    cur.execute("""
        SELECT model_size, version, COUNT(*), MIN(trained_at)::date, MAX(trained_at)::date
        FROM training_runs
        GROUP BY model_size, version
        ORDER BY model_size, version
    """)
    runs = cur.fetchall()

    if runs:
        print("Training runs:")
        for model, version, count, first, last in runs:
            if model_filter and model != model_filter:
                continue
            print(f"  {model} {version}: {count} examples ({first} to {last})")

    # Show untrained counts per model
    models = set()
    if model_filter:
        models.add(model_filter)
    else:
        cur.execute("SELECT DISTINCT model_size FROM training_runs")
        models = {r[0] for r in cur.fetchall()}
        # Always show common sizes even if no runs yet
        models.update(["4b", "9b"])

    print(f"\nUntrained examples (available for next training):")
    for model in sorted(models):
        cur.execute("""
            SELECT COUNT(*)
            FROM training_examples te
            WHERE NOT EXISTS (
                SELECT 1 FROM training_runs tr
                WHERE tr.example_id = te.id AND tr.model_size = %s
            )
        """, (model,))
        untrained = cur.fetchone()[0]
        print(f"  {model}: {untrained} untrained")

        if untrained > 0 and (model_filter == model or not model_filter):
            cur.execute("""
                SELECT dataset, COUNT(*)
                FROM training_examples te
                WHERE NOT EXISTS (
                    SELECT 1 FROM training_runs tr
                    WHERE tr.example_id = te.id AND tr.model_size = %s
                )
                GROUP BY dataset ORDER BY COUNT(*) DESC
            """, (model,))
            for dataset, count in cur.fetchall():
                print(f"    {dataset}: {count}")

    cur.close()
    conn.close()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        stats()
    elif sys.argv[1] == "--migrate":
        migrate()
    elif sys.argv[1] == "--mark-all":
        model = sys.argv[2] if len(sys.argv) > 2 else "4b"
        version = sys.argv[3] if len(sys.argv) > 3 else "v1"
        mark_all(model, version)
    elif sys.argv[1] == "--mark-new":
        model = sys.argv[2] if len(sys.argv) > 2 else "4b"
        version = sys.argv[3] if len(sys.argv) > 3 else "v2"
        mark_new(model, version)
    elif sys.argv[1] == "--stats":
        model_filter = sys.argv[2] if len(sys.argv) > 2 else None
        stats(model_filter)
    else:
        print("Usage: python mark_trained.py [--migrate|--mark-all MODEL VER|--mark-new MODEL VER|--stats [MODEL]]")
