#!/usr/bin/env bash
# Checks `seed-meeting-brief.sh seed-mailbox` / `unseed-mailbox` against a real dovecot of its own,
# on a loopback port, in a scratch directory it removes. Needs `dovecot` and python3; without
# dovecot it says so and exits 77 (skipped), so a missing server is never read as a pass.
#
#   scripts/demo/test-seed-mailbox.sh
#
# What it proves (the review of #1241, M3):
#   1. a loopback account WITHOUT the folder demo-mb-testbox is refused, and nothing is appended;
#   2. a non-loopback host is refused before any connection;
#   3. the three demo messages are appended unseen, and the password is never printed;
#   4. unseed-mailbox removes those three and leaves a foreign message that was flagged \Deleted
#      and not expunged exactly where it was.
set -u

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/seed-meeting-brief.sh"
command -v dovecot >/dev/null 2>&1 || { echo "SKIPPED: no dovecot on this machine"; exit 77; }
command -v python3 >/dev/null 2>&1 || { echo "SKIPPED: no python3"; exit 77; }

# A short path: dovecot's own sockets live under it, and a unix socket path is short.
work="$(mktemp -d /tmp/mbt.XXXXXX)"
trap 'dovecot -c "$work/dovecot.conf" stop >/dev/null 2>&1; [ -n "${KEEP:-}" ] || rm -rf "$work"' EXIT
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')"
# Dovecot will not serve mail as uid 0: run as root means serving as `nobody`.
uid="$(id -u)"; gid="$(id -g)"
if [ "$uid" = 0 ]; then uid="$(id -u nobody)"; gid="$(id -g nobody)"; fi
chmod 755 "$work"  # the auth process reads the user file as dovecot's own user
mkdir -p "$work/run" "$work/mail"
printf 'box:{PLAIN}testpass-%s\nplain:{PLAIN}testpass-%s\n' "$$" "$$" > "$work/users"
# Dovecot 2.4 (Debian 13) will not read a 2.3 file: it wants `dovecot_config_version` first and
# spells the mail location, passdb, userdb and cleartext setting differently. 2.3 and earlier keep
# the old file. The two files say the same thing: one imap listener on loopback, plaintext auth,
# a passwd-file of two users, maildir under $work/mail/<user>, and a Sent folder made on demand.
dovecot_version="$(dovecot --version 2>/dev/null | cut -d' ' -f1)"
case "$dovecot_version" in
  2.4*|2.[5-9]*|[3-9].*)
cat > "$work/dovecot.conf" <<CONF
dovecot_config_version = 2.4.0
dovecot_storage_version = 2.4.0
protocols = imap
listen = 127.0.0.1
base_dir = $work/run
log_path = $work/dovecot.log
ssl = no
auth_allow_cleartext = yes
auth_mechanisms = plain
first_valid_uid = 0
mail_driver = maildir
mail_path = $work/mail/%{user}
passdb passwd-file {
  passwd_file_path = $work/users
}
userdb static {
  fields {
    uid = $uid
    gid = $gid
    home = $work/mail/%{user}
  }
}
service imap-login {
  inet_listener imap {
    port = $port
  }
  inet_listener imaps {
    port = 0
  }
}
namespace inbox {
  inbox = yes
  mailbox Sent {
    auto = create
  }
}
CONF
    ;;
  *)
cat > "$work/dovecot.conf" <<CONF
protocols = imap
listen = 127.0.0.1
base_dir = $work/run
log_path = $work/dovecot.log
ssl = no
disable_plaintext_auth = no
auth_mechanisms = plain
first_valid_uid = 0
mail_location = maildir:$work/mail/%u
passdb {
  driver = passwd-file
  args = scheme=PLAIN username_format=%u $work/users
}
userdb {
  driver = static
  args = uid=$uid gid=$gid home=$work/mail/%u
}
service imap-login {
  inet_listener imap {
    port = $port
  }
  inet_listener imaps {
    port = 0
  }
}
namespace inbox {
  inbox = yes
  mailbox Sent {
    auto = create
  }
}
CONF
    ;;
esac
[ "$(id -u)" != 0 ] || chown -R "$uid:$gid" "$work/mail" "$work/run"
dovecot -c "$work/dovecot.conf" >"$work/start.log" 2>&1 </dev/null || { echo "dovecot would not start:"; cat "$work/start.log"; exit 1; }
for _ in $(seq 50); do
  python3 -c "import socket; socket.create_connection(('127.0.0.1', $port), 1).close()" 2>/dev/null && break
  sleep 0.1
done

pass="testpass-$$"
export MB_IMAP_PORT="$port" MB_IMAP_PASS="$pass" MB_ME=me@example.test MB_MAYA=maya@example.test MB_RAVI=ravi@example.test YANTRIK_USER_NAME=Pat

imap() { # imap <user> <python body using c>
  MB_USER="$1" MB_PORT="$port" MB_PW="$pass" python3 -c "
import imaplib, os
c = imaplib.IMAP4('127.0.0.1', int(os.environ['MB_PORT']))
c.login(os.environ['MB_USER'], os.environ['MB_PW'])
$2
c.logout()
"
}
count() { # count <user> <folder> -> messages
  imap "$1" "c.select('\"$2\"', readonly=True); print(len(c.search(None, 'ALL')[1][0].split()))"
}

failed=0
check() { if [ "$2" = "$3" ]; then echo "ok   - $1"; else echo "FAIL - $1 (got '$2', wanted '$3')"; failed=1; fi; }

# 1. A loopback account without the marker folder.
out="$(MB_IMAP_USER=plain "$script" seed-mailbox 2>&1)"; rc=$?
check "an account without demo-mb-testbox is refused" "$rc" 1
case "$out" in *demo-mb-testbox*) echo "ok   - and says why";; *) echo "FAIL - the refusal names no marker folder: $out"; failed=1;; esac
check "nothing was appended to it" "$(count plain INBOX)" 0

# The runner's account: the marker folder exists.
imap box "c.create('\"demo-mb-testbox\"'); c.create('\"Sent\"')"

# 2. A host that is not loopback is refused before a connection.
out="$(MB_IMAP_USER=box MB_IMAP_HOST=192.0.2.1 "$script" seed-mailbox 2>&1)"; rc=$?
check "a host that is not loopback is refused" "$rc" 1
case "$out" in *loopback*) echo "ok   - and says so";; *) echo "FAIL - $out"; failed=1;; esac

# 3. The seed.
out="$(MB_IMAP_USER=box "$script" seed-mailbox 2>&1)"; rc=$?
check "seed-mailbox succeeds on the runner's account" "$rc" 0
case "$out" in *"$pass"*) echo "FAIL - the password was printed"; failed=1;; *) echo "ok   - the password is not printed";; esac
check "two messages in INBOX" "$(count box INBOX)" 2
check "one in Sent" "$(count box Sent)" 1
check "all of them unseen" "$(imap box "c.select('\"INBOX\"', readonly=True); print(len(c.search(None, 'UNSEEN')[1][0].split()))")" 2

# 4. A foreign message, flagged \Deleted and not expunged, must outlive unseed-mailbox.
imap box "c.append('\"INBOX\"', '(\\\\Deleted)', None, b'From: x@example.test\r\nSubject: foreign-not-ours\r\n\r\nforeign\r\n')"
check "the foreign message is in INBOX beside the demo ones" "$(count box INBOX)" 3
out="$(MB_IMAP_USER=box "$script" unseed-mailbox 2>&1)"; rc=$?
check "unseed-mailbox succeeds" "$rc" 0
check "the foreign message is still there" "$(imap box "c.select('\"INBOX\"', readonly=True); print(len(c.search(None, 'SUBJECT', 'foreign-not-ours')[1][0].split()))")" 1
check "still flagged, not expunged" "$(imap box "c.select('\"INBOX\"', readonly=True); print(len(c.search(None, 'DELETED')[1][0].split()))")" 1
check "INBOX holds only that message" "$(count box INBOX)" 1
check "Sent holds none of the demo messages" "$(count box Sent)" 0

exit $failed
