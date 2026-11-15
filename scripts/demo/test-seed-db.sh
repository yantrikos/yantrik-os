#!/usr/bin/env bash
# Checks that `seed-meeting-brief.sh seed` keeps the mailbox rows the companion cached, and that
# `unseed` takes them back, on a scratch database and a scratch HOME. Needs python3 only.
#
#   scripts/demo/test-seed-db.sh
#
# The mailbox is seeded and cached BEFORE `seed` runs (the order the gate's list gives), and the
# brief needs those rows: a `seed` that deleted every `<demo-mb-%` row left the first brief with
# no emails.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
command -v python3 >/dev/null 2>&1 || { echo "SKIPPED: no python3"; exit 77; }

work="$(mktemp -d /tmp/seeddb.XXXXXX)"
trap 'rm -rf "$work"' EXIT
export HOME="$work/home" YANTRIK_DB="$work/memory.db" YANTRIK_USER_NAME=Pat YANTRIK_CONFIG="$work/none.yaml"
mkdir -p "$HOME"

sql() { python3 -c "
import sqlite3, sys
c = sqlite3.connect(sys.argv[1]); print(c.execute(sys.argv[2]).fetchone()[0]); c.commit()" "$YANTRIK_DB" "$1"; }
failed=0
check() { if [ "$2" = "$3" ]; then echo "ok   - $1"; else echo "FAIL - $1 (got '$2', wanted '$3')"; failed=1; fi; }

python3 -c "
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript('''
CREATE TABLE wm_commitments (id INTEGER PRIMARY KEY, promisor TEXT, promisee TEXT, action TEXT, deadline REAL, status TEXT,
  confidence REAL, source_type TEXT, source_data TEXT, evidence_text TEXT, related_entities TEXT, completion_evidence TEXT,
  created_at REAL, updated_at REAL);
CREATE TABLE prepared_items (id TEXT PRIMARY KEY);
CREATE TABLE email_accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL UNIQUE, provider TEXT NOT NULL,
  last_sync_uid INTEGER DEFAULT 0, last_sync_ts REAL DEFAULT 0);
CREATE TABLE emails (id INTEGER PRIMARY KEY, account_id INTEGER, uid INTEGER NOT NULL, folder TEXT NOT NULL DEFAULT 'INBOX',
  from_addr TEXT NOT NULL, from_name TEXT, to_addr TEXT, subject TEXT, date_ts REAL NOT NULL, body_preview TEXT, body_full TEXT,
  is_read INTEGER DEFAULT 0, is_flagged INTEGER DEFAULT 0, is_replied INTEGER DEFAULT 0, message_id TEXT, in_reply_to TEXT,
  ai_summary TEXT, importance TEXT DEFAULT 'normal', UNIQUE(account_id, uid, folder));
INSERT INTO email_accounts (name, email, provider) VALUES ('Test box', 'gate@seed.test', 'imap');
INSERT INTO emails (account_id, uid, folder, from_addr, subject, date_ts, message_id)
  VALUES (1, 1, 'INBOX', 'maya@seed.test', 'Slides for Friday', 1, '<demo-mb-box-1@seed.test>'),
         (1, 2, 'INBOX', 'x@example.test', 'Not ours', 1, '<other@example.test>');
''')" "$YANTRIK_DB"

out="$("$here/seed-meeting-brief.sh" seed 12 2>&1)"; rc=$?
check "seed succeeds" "$rc" 0
check "the mailbox's cached row survived seed" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id = '<demo-mb-box-1@seed.test>'")" 1
check "seed's own two rows are there" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id IN ('<demo-mb-1@seed.test>','<demo-mb-2@seed.test>')")" 2
"$here/seed-meeting-brief.sh" seed 12 >/dev/null 2>&1
check "a second seed does not double them" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id LIKE '<demo-mb-%'")" 3
check "two promises" "$(sql "SELECT COUNT(*) FROM wm_commitments WHERE evidence_text = 'demo-mb'")" 2

# Rows that only look like ours: another case on the test account, and the right id on another account.
python3 -c "
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript('''
INSERT INTO email_accounts (name, email, provider) VALUES ('Real', 'real@example.test', 'imap');
INSERT INTO emails (account_id, uid, folder, from_addr, subject, date_ts, message_id)
  VALUES (1, 3, 'INBOX', 'y@example.test', 'Upper', 1, '<DEMO-MB-BOX-1@seed.test>'),
         (2, 1, 'INBOX', 'z@example.test', 'Real box', 1, '<demo-mb-box-2@seed.test>');
''')" "$YANTRIK_DB"

"$here/seed-meeting-brief.sh" unseed >/dev/null 2>&1
check "unseed with no test account leaves the mailbox's cached row" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id = '<demo-mb-box-1@seed.test>'")" 1
check "but takes seed's own two" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id IN ('<demo-mb-1@seed.test>','<demo-mb-2@seed.test>')")" 0
MB_IMAP_USER=gate@seed.test "$here/seed-meeting-brief.sh" unseed >/dev/null 2>&1
check "unseed removes the test account's cached mailbox row" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id = '<demo-mb-box-1@seed.test>'")" 0
check "and not the same id in another case" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id = '<DEMO-MB-BOX-1@seed.test>'")" 1
check "and not the same id on another account" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id = '<demo-mb-box-2@seed.test>' AND account_id = 2")" 1
check "and leaves a row that is not ours" "$(sql "SELECT COUNT(*) FROM emails WHERE message_id = '<other@example.test>'")" 1
check "no promises left" "$(sql "SELECT COUNT(*) FROM wm_commitments")" 0
exit $failed
