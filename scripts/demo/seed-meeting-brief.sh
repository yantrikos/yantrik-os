#!/usr/bin/env bash
# seed-meeting-brief.sh — seed and unseed a demo meeting for the meeting brief, on a TEST machine.
#
#   seed-meeting-brief.sh seed [minutes]   a meeting that starts `minutes` from now (default 12),
#                                          two other people on it, and one email, note, promise
#                                          and file about them
#   seed-meeting-brief.sh move [minutes]   move the demo meeting's start to `minutes` from now
#                                          (a moved event must replace its card)
#   seed-meeting-brief.sh unseed           remove exactly what `seed` made, and nothing else
#   seed-meeting-brief.sh status           what is seeded now
#
# Everything made carries the marker `demo-mb`: the event's id, the note's and the file's names,
# the commitments' evidence text, the email rows' message ids and the demo account's address.
# `unseed` deletes by that marker only. A person's own rows and files are never matched.
#
# What it writes, all under $HOME or the companion's database (YANTRIK_DB, default
# /opt/yantrik/data/memory.db; the shell must have run once, so its tables exist):
#   ~/.local/share/yantrik/calendar/demo-mb-event.json   the event (the calendar service reads it)
#   ~/.local/share/yantrik/notes/demo-mb-orion-notes.md  a note whose title has "Orion" in it
#   ~/Documents/demo-orion-launch-plan.md                a file whose name has "orion" in it
#   wm_commitments                                       two open promises, one each way
#   email_accounts, emails                               a demo account and two cached messages
#
# This is test data for a machine nobody is using. Do not run it on a person's own machine.
set -euo pipefail

MARK="demo-mb"
EVENT_ID="demo-mb-event"
TITLE="Orion launch sync"
ME="demo-mb-me@seed.test"
MAYA="maya.chen@seed.test"
RAVI="ravi.rao@seed.test"
DB="${YANTRIK_DB:-/opt/yantrik/data/memory.db}"
CAL_DIR="$HOME/.local/share/yantrik/calendar"
NOTES_DIR="$HOME/.local/share/yantrik/notes"
EVENT_FILE="$CAL_DIR/$EVENT_ID.json"
NOTE_FILE="$NOTES_DIR/demo-mb-orion-notes.md"
DOC_FILE="$HOME/Documents/demo-orion-launch-plan.md"

stamp() { date -d "+$1 minutes" +%Y-%m-%dT%H:%M:%S; }

need_db() {
  [ -f "$DB" ] || { echo "no database at $DB (set YANTRIK_DB); has the shell run on this machine?" >&2; exit 1; }
  command -v python3 >/dev/null || { echo "python3 is needed to write the database" >&2; exit 1; }
}

write_event() {
  local minutes="$1"
  mkdir -p "$CAL_DIR"
  cat > "$EVENT_FILE.tmp" <<EOF
{
  "id": "$EVENT_ID",
  "title": "$TITLE",
  "description": "Agenda: the launch date, the owners, and what blocks us.\nBring the Q4 numbers.\n(demo-mb)",
  "start": "$(stamp "$minutes")",
  "end": "$(stamp $((minutes + 30)))",
  "is_all_day": false,
  "location": "Room 4",
  "attendees": [
    { "name": "Demo User", "email": "$ME", "status": "Accepted" },
    { "name": "Maya Chen", "email": "$MAYA", "status": "Accepted" },
    { "name": "Ravi Rao", "email": "$RAVI", "status": "Pending" }
  ],
  "recurrence": null,
  "calendar_id": "default",
  "remote_id": null,
  "reminder_minutes": 10
}
EOF
  mv "$EVENT_FILE.tmp" "$EVENT_FILE"
}

db() {
  python3 - "$DB" "$1" <<'PY'
import sqlite3, sys, time

path, action = sys.argv[1], sys.argv[2]
now = time.time()
con = sqlite3.connect(path, timeout=15)
con.execute("PRAGMA busy_timeout = 15000")

if action == "seed":
    con.executescript("""
        CREATE TABLE IF NOT EXISTS email_accounts (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL UNIQUE,
            provider TEXT NOT NULL, last_sync_uid INTEGER DEFAULT 0, last_sync_ts REAL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS emails (
            id INTEGER PRIMARY KEY, account_id INTEGER REFERENCES email_accounts(id),
            uid INTEGER NOT NULL, folder TEXT NOT NULL DEFAULT 'INBOX', from_addr TEXT NOT NULL,
            from_name TEXT, to_addr TEXT, subject TEXT, date_ts REAL NOT NULL, body_preview TEXT,
            body_full TEXT, is_read INTEGER DEFAULT 0, is_flagged INTEGER DEFAULT 0,
            is_replied INTEGER DEFAULT 0, message_id TEXT, in_reply_to TEXT, ai_summary TEXT,
            importance TEXT DEFAULT 'normal', UNIQUE(account_id, uid, folder));
    """)
    con.execute("INSERT OR IGNORE INTO email_accounts (name, email, provider) VALUES ('Demo (demo-mb)', ?, 'demo')", ("demo-mb-me@seed.test",))
    account = con.execute("SELECT id FROM email_accounts WHERE email = ?", ("demo-mb-me@seed.test",)).fetchone()[0]
    rows = [
        (9001, "maya.chen@seed.test", "Maya Chen", "demo-mb-me@seed.test", "Slides for Friday", now - 2 * 86400, "<demo-mb-1@seed.test>"),
        (9002, "demo-mb-me@seed.test", "Demo User", "ravi.rao@seed.test", "Q4 numbers", now - 1 * 86400, "<demo-mb-2@seed.test>"),
    ]
    for uid, frm, name, to, subject, ts, mid in rows:
        con.execute(
            "INSERT OR REPLACE INTO emails (account_id, uid, folder, from_addr, from_name, to_addr, subject, date_ts, body_preview, is_read, message_id, in_reply_to) "
            "VALUES (?, ?, 'INBOX', ?, ?, ?, ?, ?, '', 1, ?, '')",
            (account, uid, frm, name, to, subject, ts, mid),
        )
    # One promise each way; `evidence_text` carries the marker.
    for promisor, promisee, action_text in [
        ("Demo User", "Ravi Rao", "send the Q4 numbers"),
        ("Maya Chen", "Demo User", "share the deck"),
    ]:
        con.execute(
            "INSERT INTO wm_commitments (promisor, promisee, action, deadline, status, confidence, source_type, source_data, evidence_text, related_entities, completion_evidence, created_at, updated_at) "
            "VALUES (?, ?, ?, 0, 'pending', 0.9, 'manual', '\"Manual\"', 'demo-mb', '[]', NULL, ?, ?)",
            (promisor, promisee, action_text, now, now),
        )
elif action == "unseed":
    # Each delete on its own: a machine that never cached mail has no emails table, and that is
    # not a reason to leave the promises behind. Only rows carrying the marker are matched.
    for sql in (
        "DELETE FROM emails WHERE message_id LIKE '<demo-mb-%'",
        "DELETE FROM email_accounts WHERE email = 'demo-mb-me@seed.test'",
        "DELETE FROM wm_commitments WHERE evidence_text = 'demo-mb'",
        # The brief made for the demo event, so seeding again makes a fresh one.
        "DELETE FROM prepared_items WHERE id LIKE 'meeting:demo-mb-event:%'",
    ):
        try:
            con.execute(sql)
        except sqlite3.OperationalError:
            pass
elif action == "status":
    q = lambda sql: con.execute(sql).fetchone()[0]
    try:
        print("emails:", q("SELECT COUNT(*) FROM emails WHERE message_id LIKE '<demo-mb-%'"))
        print("promises:", q("SELECT COUNT(*) FROM wm_commitments WHERE evidence_text = 'demo-mb'"))
        print("briefs:", q("SELECT COUNT(*) FROM prepared_items WHERE id LIKE 'meeting:demo-mb-event:%'"))
    except sqlite3.OperationalError as e:
        print("database:", e)
con.commit()
con.close()
PY
}

case "${1:-}" in
  seed)
    need_db
    minutes="${2:-12}"
    db unseed >/dev/null
    write_event "$minutes"
    mkdir -p "$NOTES_DIR" "$HOME/Documents"
    printf '# Orion launch notes (demo-mb)\n\nMet Maya Chen and Ravi Rao about the launch date.\n' > "$NOTE_FILE"
    printf 'Orion launch plan (demo-mb)\n' > "$DOC_FILE"
    db seed
    echo "seeded: '$TITLE' starts $(stamp "$minutes") with Maya Chen and Ravi Rao; the card should appear at T-10."
    ;;
  move)
    [ -f "$EVENT_FILE" ] || { echo "nothing seeded; run seed first" >&2; exit 1; }
    minutes="${2:-9}"
    write_event "$minutes"
    echo "moved: '$TITLE' now starts $(stamp "$minutes")."
    ;;
  unseed)
    rm -f "$EVENT_FILE" "$EVENT_FILE.tmp" "$NOTE_FILE" "$DOC_FILE"
    if [ -f "$DB" ] && command -v python3 >/dev/null; then db unseed; fi
    echo "unseeded: everything marked $MARK is gone."
    ;;
  status)
    for f in "$EVENT_FILE" "$NOTE_FILE" "$DOC_FILE"; do
      if [ -e "$f" ]; then echo "present: $f"; else echo "absent:  $f"; fi
    done
    if [ -f "$DB" ] && command -v python3 >/dev/null; then db status; fi
    ;;
  *)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
