#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# sign-manifest.sh — write the signed release manifest for one channel (#537)
# ═══════════════════════════════════════════════════════════════════════════════════════
#
#   sign-manifest.sh --channel C --version V --git G --artifact NAME --sha256 H [--binaries N]
#                    (--key-env VAR | --key-file PATH) --out FILE
#                    [--serial N] [--valid-days D] [--now EPOCH]
#
# Produces the file yantrik-update-verify reads (its header describes the format): one
# envelope, the signature on its first line and the exact manifest bytes after it.
#
# The key is an ed25519 private key in PEM, made by update-keys/new-key.sh. It comes from the
# environment variable named by --key-env (a CI secret) or from the file --key-file; it is never
# given on a command line, never printed, and the scratch copy made for openssl is mode 0600 and
# removed on exit. Which key signs which channel is decided by the machine, not here: a nightly
# key signs a stable manifest without complaint, and the updater refuses it.
#
# --serial defaults to the time of signing, in seconds. It only has to rise from one release to
# the next on a channel, which a clock does without any state to keep or lose; pass one
# explicitly to re-sign a release or to issue a deliberate correction.
# --valid-days (default 60) is how long machines will believe the file. A channel nobody
# publishes to for that long stops updating machines with a sentence saying so, rather than
# leaving them to trust an arbitrarily old file.
set -euo pipefail

fail() { echo "sign-manifest.sh: $*" >&2; exit 1; }

CHANNEL=""; VERSION=""; GIT=""; ARTIFACT=""; SHA=""; BINARIES=""; KEY_ENV=""; KEY_FILE=""
OUT=""; SERIAL=""; DAYS=60; NOW=""
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || fail "$1 needs a value"
  case "$1" in
    --channel) CHANNEL="$2" ;;   --version) VERSION="$2" ;;   --git) GIT="$2" ;;
    --artifact) ARTIFACT="$2" ;; --sha256) SHA="$2" ;;        --binaries) BINARIES="$2" ;;
    --key-env) KEY_ENV="$2" ;;   --key-file) KEY_FILE="$2" ;; --out) OUT="$2" ;;
    --serial) SERIAL="$2" ;;     --valid-days) DAYS="$2" ;;   --now) NOW="$2" ;;
    *) fail "unknown argument $1" ;;
  esac
  shift 2
done

case "$CHANNEL" in nightly|beta|stable) ;; *) fail "--channel is nightly, beta or stable" ;; esac
word='^[A-Za-z0-9][A-Za-z0-9._+-]{0,120}$'
for pair in "version:$VERSION" "git:$GIT" "artifact:$ARTIFACT"; do
  printf '%s' "${pair#*:}" | grep -qE "$word" || fail "--${pair%%:*} is missing or has characters a release name never has"
done
printf '%s' "$SHA" | grep -qE '^[0-9a-f]{64}$' || fail "--sha256 is not a sha256"
[ -z "$BINARIES" ] || printf '%s' "$BINARIES" | grep -qE '^[0-9]+$' || fail "--binaries is a count"
[ -n "$OUT" ] || fail "--out is required"
[ -n "$NOW" ] || NOW="$(date +%s)"
[ -n "$SERIAL" ] || SERIAL="$NOW"
for n in "$SERIAL" "$DAYS" "$NOW"; do
  printf '%s' "$n" | grep -qE '^[0-9]+$' || fail "--serial, --valid-days and --now are whole numbers"
done
[ "$DAYS" -ge 1 ] && [ "$DAYS" -le 400 ] || fail "--valid-days is between 1 and 400"
command -v openssl >/dev/null 2>&1 || fail "openssl 3 is required"

if [ -n "$KEY_ENV" ] && [ -n "$KEY_FILE" ]; then fail "give --key-env or --key-file, not both"; fi
[ -n "$KEY_ENV" ] || [ -n "$KEY_FILE" ] || fail "give --key-env VAR or --key-file PATH"

WORK="$(umask 077 && mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
KEY="$WORK/key.pem"
if [ -n "$KEY_ENV" ]; then
  printf '%s' "$KEY_ENV" | grep -qE '^[A-Za-z_][A-Za-z0-9_]*$' || fail "--key-env is an environment variable name"
  [ -n "${!KEY_ENV:-}" ] || fail "\$$KEY_ENV is empty or unset: nothing to sign with"
  ( umask 077; printf '%s\n' "${!KEY_ENV}" > "$KEY" )
else
  [ -f "$KEY_FILE" ] || fail "no key file at $KEY_FILE"
  ( umask 077; cp "$KEY_FILE" "$KEY" )
fi
openssl pkey -in "$KEY" -text -noout 2>/dev/null | head -n 1 | grep -qi 'ED25519' \
  || fail "that is not an ed25519 private key"

# The manifest, as bytes. Built once, signed as written, and never touched again.
python3 -I - "$WORK/body" "$CHANNEL" "$SERIAL" "$NOW" "$DAYS" "$VERSION" "$GIT" "$ARTIFACT" "$SHA" "$BINARIES" <<'PY'
import json, sys, time
out, channel, serial, now, days, version, git, artifact, sha, binaries = sys.argv[1:]
now, days = int(now), int(days)
stamp = lambda t: time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t))
m = {
    "schema": 1, "channel": channel, "serial": int(serial),
    "issued": stamp(now), "expires": stamp(now + days * 86400),
    "version": version, "git": git, "artifact": artifact, "sha256": sha,
    "date": time.strftime("%Y-%m-%d", time.gmtime(now)),
}
if binaries:
    m["binaries"] = int(binaries)
open(out, "w", newline="\n").write(json.dumps(m, indent=2) + "\n")
PY

openssl pkeyutl -sign -rawin -inkey "$KEY" -in "$WORK/body" -out "$WORK/sig" 2>/dev/null \
  || fail "openssl could not sign (it needs OpenSSL 3 for ed25519)"
# Verified against the key's own public half before anything is written: a signature that
# does not verify is caught here, on the machine that made it, and not by every machine at once.
openssl pkey -in "$KEY" -pubout -out "$WORK/pub.pem" 2>/dev/null
openssl pkeyutl -verify -pubin -inkey "$WORK/pub.pem" -rawin -in "$WORK/body" -sigfile "$WORK/sig" >/dev/null 2>&1 \
  || fail "the signature did not verify against its own key"

{ printf 'yantrik-manifest v1 ed25519 %s\n' "$(openssl base64 -A -in "$WORK/sig")"; cat "$WORK/body"; } > "$OUT.part"
mv "$OUT.part" "$OUT"
echo "signed $CHANNEL serial $SERIAL (valid $DAYS days) -> $OUT"
