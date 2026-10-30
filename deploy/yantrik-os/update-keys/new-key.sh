#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# new-key.sh — make an update-signing key (#537)
# ═══════════════════════════════════════════════════════════════════════════════════════
#
#   new-key.sh <nightly|release> <id> <private-key-path>
#
# Writes the ed25519 private key to <private-key-path>, mode 0600, and refuses to overwrite one
# that is there. Prints the PUBLIC key and the file name to commit it under, and nothing else:
# the private key is never printed, never put in an environment variable by this script and
# never written anywhere but the path you chose.
#
#   nightly   signs the nightly channel only. Goes into the repository secret
#             UPDATE_SIGNING_KEY_NIGHTLY (`gh secret set … < the private key file`), so the
#             pipeline can publish a nightly by itself.
#   release   signs beta and stable. Keep it OFFLINE (a hardware token or an encrypted volume).
#             CI never has it; a beta or stable manifest is signed by hand with sign-manifest.sh.
#
# The public half is committed here as `<scope>.<id>.pub` and ships in every build under
# share/update-keys. Two keys of one scope can be trusted at once, which is a rotation: make the
# new key with a new <id>, ship it, switch the signing over, delete the old .pub a release later.
set -euo pipefail

fail() { echo "new-key.sh: $*" >&2; exit 1; }
[ $# -eq 3 ] || fail "usage: new-key.sh <nightly|release> <id> <private-key-path>"
SCOPE="$1"; ID="$2"; PRIV="$3"
case "$SCOPE" in nightly|release) ;; *) fail "the scope is nightly or release" ;; esac
printf '%s' "$ID" | grep -qE '^[A-Za-z0-9_-]{1,32}$' || fail "the id is letters, digits, - and _ (up to 32)"
[ ! -e "$PRIV" ] || fail "$PRIV already exists; not overwriting a key"
[ -d "$(dirname "$PRIV")" ] || fail "no directory for $PRIV"
command -v openssl >/dev/null 2>&1 || fail "openssl 3 is required"

( umask 077; openssl genpkey -algorithm ed25519 -out "$PRIV" ) 2>/dev/null \
  || fail "openssl could not make an ed25519 key (it needs OpenSSL 3)"
chmod 600 "$PRIV"

echo "Private key written to $PRIV (mode 0600). It is not shown and not stored anywhere else."
echo "Commit this as deploy/yantrik-os/update-keys/$SCOPE.$ID.pub :"
echo
openssl pkey -in "$PRIV" -pubout
