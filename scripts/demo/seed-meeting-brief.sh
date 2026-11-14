#!/usr/bin/env bash
# seed-meeting-brief.sh — seed and unseed a demo meeting for the meeting brief, on a TEST machine.
#
#   seed-meeting-brief.sh seed [minutes]   a meeting that starts `minutes` from now (default 12),
#                                          two other people on it, and one note, promise and file
#                                          about them, and (unless MB_DEMO_MAIL=0) two demo emails
#   seed-meeting-brief.sh move [minutes]   move the demo meeting's start to `minutes` from now
#                                          (a moved event must replace its card)
#   seed-meeting-brief.sh unseed           remove exactly what `seed` made, and nothing else
#   seed-meeting-brief.sh status           what is seeded now
#   seed-meeting-brief.sh seed-mailbox     three messages in a LOOPBACK test mailbox, over IMAP APPEND
#   seed-meeting-brief.sh unseed-mailbox   remove exactly those messages from it
#
# Everything made carries the marker `demo-mb`: the event's id, the note's and the file's names,
# the commitments' evidence text, the email rows' message ids and the demo account's address, and
# the `X-Demo-MB` header of a mailbox message. `unseed` deletes by that marker only. A person's
# own rows, files and mail are never matched.
#
# What `seed` writes, all under $HOME or the companion's database (YANTRIK_DB, default
# /opt/yantrik/data/memory.db; the shell must have run once, so its tables exist):
#   ~/.local/share/yantrik/calendar/demo-mb-event.json   the event (the calendar service reads it)
#   ~/.local/share/yantrik/notes/demo-mb-orion-notes.md  a note whose title has "Orion" in it
#   ~/Documents/demo-mb-orion-launch-plan.md             a file whose name has "orion" in it
#                                                        (refused if a file of that name is
#                                                        already there and is not this script's)
#   wm_commitments                                       two open promises, one each way
#   email_accounts, emails                               a demo account and two cached messages
#                                                        (nothing syncs from this table: the email
#                                                        sync reads the configured accounts, so an
#                                                        account of provider "demo" is never dialled;
#                                                        and a message in it cannot be opened, for it
#                                                        is on no server. MB_DEMO_MAIL=0 leaves it out,
#                                                        for a run with a real test mailbox)
#
# The people on the event are the person (MB_ME), Maya Chen (MB_MAYA) and Ravi Rao (MB_RAVI). The
# defaults are addresses on seed.test. With a test mailbox, set them to the addresses the mailbox's
# messages are from and to, so the brief finds those messages.
#
# The two promises are between the person and the other people, so they are written with the
# person's own name: YANTRIK_USER_NAME, or `user_name:` in YANTRIK_CONFIG (default
# /opt/yantrik/config.yaml). With neither, `seed` stops: promises under another name would not
# be the person's and would not show.
#
# The test mailbox (`seed-mailbox`, `unseed-mailbox`). This installs and starts nothing: it
# assumes an IMAP server the runner already runs, on this machine's loopback (dovecot, say), with
# an account made for the test. It never touches a person's real mailbox: a host that is not a
# loopback address is refused before anything is sent, and nothing is read or marked beyond the
# `X-Demo-MB` messages it made itself. Settings, all from the environment:
#   MB_IMAP_USER        the test account's user (needed)
#   MB_IMAP_PASS        its password, or MB_IMAP_PASS_FILE: a file holding it. Never printed.
#   MB_IMAP_HOST        default 127.0.0.1        MB_IMAP_PORT   default 143
#   MB_IMAP_SSL=1       IMAPS, for a server that wants it (default: plain, on loopback)
# It makes: Maya's "Slides for Friday" in INBOX (to the person), the person's "Q4 numbers" in
# Sent (to Ravi), and Maya's decoy "Slides for Friday v2" in INBOX. All three arrive unseen.
#
# This is test data for a machine nobody is using. Do not run it on a person's own machine.
set -euo pipefail

MARK="demo-mb"
EVENT_ID="demo-mb-event"
TITLE="Orion launch sync"
export MB_ME="${MB_ME:-demo-mb-me@seed.test}"
export MB_MAYA="${MB_MAYA:-maya.chen@seed.test}"
export MB_RAVI="${MB_RAVI:-ravi.rao@seed.test}"
DB="${YANTRIK_DB:-/opt/yantrik/data/memory.db}"
CAL_DIR="$HOME/.local/share/yantrik/calendar"
NOTES_DIR="$HOME/.local/share/yantrik/notes"
EVENT_FILE="$CAL_DIR/$EVENT_ID.json"
NOTE_FILE="$NOTES_DIR/demo-mb-orion-notes.md"
DOC_FILE="$HOME/Documents/demo-mb-orion-launch-plan.md"
CONFIG="${YANTRIK_CONFIG:-/opt/yantrik/config.yaml}"

user_name() {
  if [ -n "${YANTRIK_USER_NAME:-}" ]; then printf '%s' "$YANTRIK_USER_NAME"; return; fi
  [ -f "$CONFIG" ] || return 0
  sed -n 's/^[[:space:]]*user_name:[[:space:]]*//p' "$CONFIG" | head -1 | sed -e 's/^["'"'"']//' -e 's/["'"'"'][[:space:]]*$//'
}

stamp() { date -d "+$1 minutes" +%Y-%m-%dT%H:%M:%S; }

need_python() {
  command -v python3 >/dev/null || { echo "python3 is needed" >&2; exit 1; }
}

need_db() {
  [ -f "$DB" ] || { echo "no database at $DB (set YANTRIK_DB); has the shell run on this machine?" >&2; exit 1; }
  need_python
}

# The event file, written by json.dumps: a name with a quote or a backslash in it is still valid
# JSON, where a hand-written file would be one the calendar skips without a word.
write_event() {
  local minutes="$1"
  mkdir -p "$CAL_DIR"
  python3 - "$EVENT_FILE.tmp" "$EVENT_ID" "$TITLE" "$(stamp "$minutes")" "$(stamp $((minutes + 30)))" "$(user_name)" <<'PY'
import json, os, sys

path, event_id, title, start, end, person = sys.argv[1:7]
event = {
    "id": event_id,
    "title": title,
    "description": "Agenda: the launch date, the owners, and what blocks us.\nBring the Q4 numbers.\n(demo-mb)",
    "start": start,
    "end": end,
    "is_all_day": False,
    "location": "Room 4",
    "attendees": [
        {"name": person, "email": os.environ["MB_ME"], "status": "Accepted"},
        {"name": "Maya Chen", "email": os.environ["MB_MAYA"], "status": "Accepted"},
        {"name": "Ravi Rao", "email": os.environ["MB_RAVI"], "status": "Pending"},
    ],
    "recurrence": None,
    "calendar_id": "default",
    "remote_id": None,
    "reminder_minutes": 10,
}
with open(path, "w", encoding="utf-8") as f:
    json.dump(event, f, indent=2)
PY
  mv "$EVENT_FILE.tmp" "$EVENT_FILE"
}

db() {
  python3 - "$DB" "$1" "$(user_name)" "${MB_DEMO_MAIL:-1}" <<'PY'
import os, sqlite3, sys, time

path, action, person, demo_mail = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4] != "0"
now = time.time()
con = sqlite3.connect(path, timeout=15)
con.execute("PRAGMA busy_timeout = 15000")
me, maya, ravi = os.environ["MB_ME"], os.environ["MB_MAYA"], os.environ["MB_RAVI"]
# The demo account is its own address, never the person's configured one.
demo_account = "demo-mb-me@seed.test"

if action == "seed":
    if demo_mail:
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
        con.execute("INSERT OR IGNORE INTO email_accounts (name, email, provider) VALUES ('Demo (demo-mb)', ?, 'demo')", (demo_account,))
        account = con.execute("SELECT id FROM email_accounts WHERE email = ?", (demo_account,)).fetchone()[0]
        rows = [
            (9001, "INBOX", maya, "Maya Chen", me, "Slides for Friday", now - 2 * 86400, "<demo-mb-1@seed.test>"),
            # The person's own mail lives in Sent: its row must open from there.
            (9002, "Sent", me, person, ravi, "Q4 numbers", now - 1 * 86400, "<demo-mb-2@seed.test>"),
        ]
        for uid, folder, frm, name, to, subject, ts, mid in rows:
            con.execute(
                "INSERT OR REPLACE INTO emails (account_id, uid, folder, from_addr, from_name, to_addr, subject, date_ts, body_preview, is_read, message_id, in_reply_to) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, '', 1, ?, '')",
                (account, uid, folder, frm, name, to, subject, ts, mid),
            )
    # One promise each way; `evidence_text` carries the marker.
    for promisor, promisee, action_text in [
        (person, "Ravi Rao", "send the Q4 numbers"),
        ("Maya Chen", person, "share the deck"),
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

# The test mailbox: three messages over IMAP APPEND, or the removal of exactly those. The server is
# one the runner started on this machine's loopback; a host that is not a loopback address is
# refused before a connection is made, so a person's own account cannot be reached by a mistake in
# the environment. Only messages carrying the X-Demo-MB header are ever searched for or removed.
mailbox() {
  need_python
  [ -n "${MB_IMAP_USER:-}" ] || { echo "no test mailbox: set MB_IMAP_USER (and MB_IMAP_PASS or MB_IMAP_PASS_FILE)" >&2; exit 1; }
  python3 - "$1" "$(user_name)" <<'PY'
import email.message, email.utils, imaplib, ipaddress, os, socket, sys, time

action, person = sys.argv[1], sys.argv[2]
host = os.environ.get("MB_IMAP_HOST", "127.0.0.1")
port = int(os.environ.get("MB_IMAP_PORT", "143"))
secure = os.environ.get("MB_IMAP_SSL", "0") == "1"
user = os.environ["MB_IMAP_USER"]
me, maya, ravi = os.environ["MB_ME"], os.environ["MB_MAYA"], os.environ["MB_RAVI"]

def fail(why):
    print(why, file=sys.stderr)
    sys.exit(1)

# Loopback only, judged on what the name resolves to, not on how it is spelled.
try:
    found = {info[4][0] for info in socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)}
except OSError as e:
    fail(f"cannot resolve MB_IMAP_HOST: {e}")
if not found or not all(ipaddress.ip_address(a.split('%')[0]).is_loopback for a in found):
    fail("MB_IMAP_HOST is not a loopback address; this seeds a test mailbox on this machine only, "
         "never a person's own account")

password = os.environ.get("MB_IMAP_PASS", "")
if not password and os.environ.get("MB_IMAP_PASS_FILE"):
    try:
        password = open(os.environ["MB_IMAP_PASS_FILE"], encoding="utf-8").read().strip()
    except OSError as e:
        fail(f"cannot read MB_IMAP_PASS_FILE: {e}")
if not password:
    fail("no password: set MB_IMAP_PASS or MB_IMAP_PASS_FILE")

try:
    conn = (imaplib.IMAP4_SSL if secure else imaplib.IMAP4)(host, port)
except (OSError, imaplib.IMAP4.error) as e:
    fail(f"no IMAP server answers on {host}:{port} ({getattr(e, 'strerror', None) or e}); the runner starts one, this does not")
try:
    conn.login(user, password)
except imaplib.IMAP4.error:
    fail("the test mailbox refused the login")

def made(folder):
    """The message numbers in `folder` this script made, by its header."""
    if conn.select(f'"{folder}"')[0] != "OK":
        return []
    kind, data = conn.search(None, "HEADER", "X-Demo-MB", "demo-mb")
    return data[0].split() if kind == "OK" and data and data[0] else []

def remove():
    gone = 0
    for folder in ("INBOX", "Sent"):
        numbers = made(folder)
        for n in numbers:
            conn.store(n, "+FLAGS", "(\\Deleted)")
        if numbers:
            conn.expunge()
        gone += len(numbers)
    return gone

def build(n, sender_name, sender, to_name, to, subject, age_days):
    msg = email.message.EmailMessage()
    msg["From"] = email.utils.formataddr((sender_name, sender))
    msg["To"] = email.utils.formataddr((to_name, to))
    msg["Subject"] = subject
    when = time.time() - age_days * 86400
    msg["Date"] = email.utils.formatdate(when)
    msg["Message-ID"] = f"<demo-mb-box-{n}@seed.test>"
    msg["X-Demo-MB"] = "demo-mb"
    msg.set_content(f"{subject}\n\n(demo-mb)\n")
    return msg, when

try:
    if action == "unseed-mailbox":
        print(f"removed {remove()} demo message(s) from the test mailbox")
    else:
        remove()
        conn.create('"Sent"')  # already there is an answer too
        plan = [
            ("INBOX", build(1, "Maya Chen", maya, person, me, "Slides for Friday", 2)),
            ("Sent", build(2, person, me, "Ravi Rao", ravi, "Q4 numbers", 1)),
            ("INBOX", build(3, "Maya Chen", maya, person, me, "Slides for Friday v2", 3)),
        ]
        for folder, (msg, when) in plan:
            # No flags: the message arrives unseen, and the gate checks it still is afterwards.
            kind, _ = conn.append(f'"{folder}"', None, imaplib.Time2Internaldate(when), msg.as_bytes())
            if kind != "OK":
                fail(f"the test mailbox would not take a message into {folder}")
        print("seeded the test mailbox: Slides for Friday and Slides for Friday v2 (INBOX), Q4 numbers (Sent), all unseen")
finally:
    try:
        conn.logout()
    except Exception:
        pass
PY
}

case "${1:-}" in
  seed)
    need_db
    minutes="${2:-12}"
    [ -n "$(user_name)" ] || { echo "no user name: set YANTRIK_USER_NAME (or user_name: in $CONFIG)" >&2; exit 1; }
    if [ -e "$DOC_FILE" ] && ! grep -q "$MARK" "$DOC_FILE"; then
      echo "$DOC_FILE exists and is not this script's; not overwriting it" >&2
      exit 1
    fi
    db unseed >/dev/null
    write_event "$minutes"
    mkdir -p "$NOTES_DIR" "$HOME/Documents"
    printf '# Orion launch notes (demo-mb)\n\nMet Maya Chen and Ravi Rao about the launch date.\n' > "$NOTE_FILE"
    printf 'Orion launch plan (demo-mb)\n' > "$DOC_FILE"
    db seed
    echo "seeded: '$TITLE' starts $(stamp "$minutes") with Maya Chen and Ravi Rao; the card should appear at T-10."
    ;;
  move)
    need_python
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
  seed-mailbox)
    [ -n "$(user_name)" ] || { echo "no user name: set YANTRIK_USER_NAME (or user_name: in $CONFIG)" >&2; exit 1; }
    mailbox seed-mailbox
    ;;
  unseed-mailbox)
    mailbox unseed-mailbox
    ;;
  *)
    sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
