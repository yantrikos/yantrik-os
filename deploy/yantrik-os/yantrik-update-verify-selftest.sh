#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# yantrik-update-verify-selftest.sh — the signed update manifest, end to end (#537)
# ═══════════════════════════════════════════════════════════════════════════════════════
#
#   bash deploy/yantrik-os/yantrik-update-verify-selftest.sh
#
# Throwaway keys are made here with openssl and thrown away; no real key, secret or server is
# touched. It runs the real sign-manifest.sh, new-key.sh and yantrik-update-verify, and then
# the real `yantrik-update check` against a local web server, in a scratch tree that pretends
# to be root's. Needs bash, openssl 3, python3, curl-less (the updater fetches with python).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VERIFY="$HERE/yantrik-update-verify"
SIGN="$HERE/sign-manifest.sh"
NEWKEY="$HERE/update-keys/new-key.sh"
UPDATE="$HERE/yantrik-update"

T="$(mktemp -d)"
SERVER_PID=""
cleanup() { [ -z "$SERVER_PID" ] || kill "$SERVER_PID" 2>/dev/null; rm -rf "$T"; }
trap cleanup EXIT

fails=0
ok()   { printf '  ok   %s\n' "$1"; }
bad()  { printf '  FAIL %s\n       %s\n' "$1" "$2"; fails=$((fails + 1)); }
is()   { if [ "$3" = "$2" ]; then ok "$1"; else bad "$1" "wanted: $2 / got: $3"; fi; } # name want got
has()  { if printf '%s' "$3" | grep -qF -- "$2"; then ok "$1"; else bad "$1" "wanted: $2 / got: $(printf '%s' "$3" | tr '\n' ' ')"; fi; }
hasnt() { if printf '%s' "$3" | grep -qF -- "$2"; then bad "$1" "found: $2"; else ok "$1"; fi; }

# The scratch tree stands in for /opt/yantrik. Its owner is whoever runs this, so the verifier
# is told (selftest only, never as root) that this uid is "root" for the ownership checks.
export YANTRIK_SELFTEST=1
export YANTRIK_VERIFY_ROOT_UID="$(id -u)"
PFX="$T/opt"; KEYS="$PFX/share/update-keys"; STATE="$PFX/update-state"
mkdir -p "$KEYS" "$PFX/bin"
chmod 755 "$PFX" "$PFX/share" "$KEYS"

# Keys. nightly.t1 may sign nightly only, release.t1 beta and stable; rogue is trusted by nobody.
mkkey() { "$NEWKEY" "$1" "$2" "$T/$2.key" > "$T/$2.out" 2>&1 || { cat "$T/$2.out"; exit 1; }; }
mkkey nightly n1; mkkey release r1; mkkey release rogue
openssl pkey -in "$T/n1.key" -pubout -out "$KEYS/nightly.n1.pub"
openssl pkey -in "$T/r1.key" -pubout -out "$KEYS/release.r1.pub"
chmod 644 "$KEYS"/*.pub
cp "$KEYS/nightly.n1.pub" "$T/n1.pub"
# The list of what the installed build shipped (bin/yantrik-root-files.sha256): a key counts only
# when it is on it. `relist` is what a build's packaging does.
LIST="$PFX/bin/yantrik-root-files.sha256"
relist() { (cd "$(dirname "$KEYS")" && sha256sum update-keys/*) > "$LIST"; chmod 644 "$LIST"; }
relist

SHA="$(printf 'bundle' | sha256sum | cut -d' ' -f1)"
NOW=1800000000   # a fixed clock, so expiry cases do not depend on the day this runs
sign() { # <out> <channel> <serial> <key> [days]
  "$SIGN" --channel "$2" --version v1.2.3 --git abc1234 --artifact yantrik-os-v1.2.3-linux-amd64.tar.zst \
    --sha256 "$SHA" --binaries 30 --key-file "$T/$4.key" --out "$1" --serial "$3" \
    --valid-days "${5:-60}" --now "$NOW" > /dev/null
}
verify() { # <channel> <file> [extra args]: prints the verifier's answer; sets $rc
  local ch="$1" f="$2"; shift 2
  out="$("$VERIFY" verify --channel "$ch" --source releases.example --keys "$KEYS" --state "$STATE" --listed "$LIST" --now "$((NOW + 86400))" "$@" "$f" 2>&1)"; rc=$?
}
field() { printf '%s\n' "$out" | sed -n "s/^$1=//p" | head -1; }

echo "yantrik-update-verify selftest"

# ── 1. A good signature, and nothing read before it ─────────────────────────────────────
sign "$T/n100" nightly 100 n1
verify nightly "$T/n100"
is  "a valid nightly signature is accepted" "ok:0" "$(field status):$rc"
is  "and the fields come out of the signed manifest" "v1.2.3 abc1234 100" "$(field version) $(field git) $(field serial)"
is  "the bundle hash is the signed one" "$SHA" "$(field sha256)"
is  "a good check changes nothing without --commit" "no" "$([ -e "$STATE/nightly.serial" ] && echo yes || echo no)"
sign "$T/r100" stable 100 r1
verify stable "$T/r100"
is  "the offline release key signs stable" "ok:0" "$(field status):$rc"
sign "$T/rb" beta 100 r1
verify beta "$T/rb"
is  "and beta" "ok:0" "$(field status):$rc"

# ── 2. Tampering, wrong keys, wrong scope ───────────────────────────────────────────────
sed 's/abc1234/abc1235/' "$T/n100" > "$T/tampered"
verify nightly "$T/tampered"
is  "one changed byte in the manifest is refused" "bad_signature:20" "$(field status):$rc"
has "with a sentence a person can read" "is not signed by Yantrik, so nothing was installed" "$(field detail)"
# The first line is the signature: change one character of it instead.
first="$(sed -n '1s/^yantrik-manifest v1 ed25519 \(.\).*/\1/p' "$T/n100")"; swap=A; [ "$first" != A ] || swap=B
sed "1s/^\(yantrik-manifest v1 ed25519 \)./\1$swap/" "$T/n100" > "$T/sigtamper"
verify nightly "$T/sigtamper"
is  "a changed signature is refused" "20" "$rc"
sign "$T/rogue" nightly 100 rogue
verify nightly "$T/rogue"
is  "a key nobody trusts is refused" "bad_signature:20" "$(field status):$rc"
sign "$T/n-stable" stable 100 n1
verify stable "$T/n-stable"
is  "the nightly key signing stable is refused" "key_scope:20" "$(field status):$rc"
has "and says why" "only allowed to sign other channels" "$(field detail)"
sign "$T/r-nightly" nightly 100 r1
verify nightly "$T/r-nightly"
is  "the release key does not sign nightly either" "key_scope:20" "$(field status):$rc"
verify stable "$T/n100"
is  "a nightly manifest served as stable is refused" "key_scope:20" "$(field status):$rc"
verify stable "$T/rb"
is  "a beta manifest served as stable is refused, though the key is allowed" "channel_mismatch:20" "$(field status):$rc"

# ── 3. Unsigned, empty, malformed ───────────────────────────────────────────────────────
printf '{"channels":{"nightly":{"version":"v9","artifact":"x.tar.zst","sha256":"%s"}}}\n' "$SHA" > "$T/legacy.json"
verify nightly "$T/legacy.json"
is  "the old unsigned manifest is refused" "unsigned:20" "$(field status):$rc"
has "in the sentence the issue asks for" "The update information from releases.example is not signed by Yantrik, so nothing was installed" "$(field detail)"
: > "$T/empty"
verify nightly "$T/empty"
is  "an empty file is refused" "unsigned:20" "$(field status):$rc"
verify nightly "$T/does-not-exist"
is  "a missing file is refused" "unsigned:20" "$(field status):$rc"
{ head -n 1 "$T/n100"; printf 'not json\n'; } > "$T/garbled"
verify nightly "$T/garbled"
is  "a bad signature on garbage is refused before the garbage is read" "bad_signature:20" "$(field status):$rc"
printf 'not json\n' > "$T/garbage-body"
openssl pkeyutl -sign -rawin -inkey "$T/n1.key" -in "$T/garbage-body" -out "$T/garbage.sig" 2>/dev/null
{ printf 'yantrik-manifest v1 ed25519 %s\n' "$(openssl base64 -A -in "$T/garbage.sig")"; cat "$T/garbage-body"; } > "$T/signed-garbage"
verify nightly "$T/signed-garbage"
is  "validly signed garbage is malformed, not accepted" "malformed:20" "$(field status):$rc"

# ── 4. Expiry ───────────────────────────────────────────────────────────────────────────
sign "$T/short" nightly 101 n1 1
verify nightly "$T/short" --now "$((NOW + 3 * 86400))"
is  "an expired manifest is refused" "expired:20" "$(field status):$rc"
has "with the date and what to do" "expired on 2027-01-16" "$(field detail)"
verify nightly "$T/short" --now "$((NOW + 3600))"
is  "the same manifest an hour after signing is fine" "ok:0" "$(field status):$rc"

# ── 5. Serial: no rollback by replay, last-good kept ────────────────────────────────────
rm -rf "$STATE"
sign "$T/n200" nightly 200 n1
verify nightly "$T/n200" --commit
is  "a manifest is accepted and committed" "ok:0" "$(field status):$rc"
is  "the serial is recorded" "200" "$(cut -d' ' -f1 "$STATE/nightly.serial")"
cp "$STATE/nightly.serial" "$T/serial.before"; cp "$STATE/nightly.last-good" "$T/lastgood.before"
verify nightly "$T/n100" --commit
is  "an older, validly signed manifest is refused" "replay:20" "$(field status):$rc"
has "as a possible replay" "older (serial 100) than one this machine already accepted (serial 200)" "$(field detail)"
sign "$T/n200b" nightly 200 n1 30
verify nightly "$T/n200b" --commit
is  "the same serial with different content is refused" "replay:20" "$(field status):$rc"
verify nightly "$T/tampered" --commit
verify nightly "$T/legacy.json" --commit
verify nightly "$T/short" --commit --now "$((NOW + 9 * 86400))"
is  "the recorded serial survived every refusal" "yes" "$(cmp -s "$STATE/nightly.serial" "$T/serial.before" && echo yes || echo no)"
is  "and so did the last-good manifest" "yes" "$(cmp -s "$STATE/nightly.last-good" "$T/lastgood.before" && echo yes || echo no)"
is  "the last-good file is the accepted envelope itself" "yes" "$(cmp -s "$STATE/nightly.last-good" "$T/n200" && echo yes || echo no)"
verify nightly "$T/n200"
is  "re-reading the accepted manifest still works" "ok:0" "$(field status):$rc"
sign "$T/n300" nightly 300 n1
verify nightly "$T/n300" --commit
is  "a higher serial moves the record forward" "300" "$(cut -d' ' -f1 "$STATE/nightly.serial")"
is  "per channel: stable has its own record" "no" "$([ -e "$STATE/stable.serial" ] && echo yes || echo no)"
printf 'garbage\n' > "$STATE/nightly.serial"
verify nightly "$T/n300"
is  "an unreadable record is a refusal, not a reset" "state_unreadable:20" "$(field status):$rc"
rm -rf "$STATE"

# ── 6. Which keys are trusted ───────────────────────────────────────────────────────────
chmod 777 "$KEYS"
verify nightly "$T/n100"
is  "a key directory anyone can write is not trusted" "keys_untrusted:20" "$(field status):$rc"
chmod 755 "$KEYS"
mv "$KEYS" "$PFX/share/keys.away"; mkdir "$KEYS"; chmod 755 "$KEYS"
verify nightly "$T/n100"
is  "no keys is a refusal" "no_keys:20" "$(field status):$rc"
rmdir "$KEYS"; mv "$PFX/share/keys.away" "$KEYS"
# A second key of a scope is trusted too: that is how a key is rotated.
mkkey nightly n2; openssl pkey -in "$T/n2.key" -pubout -out "$KEYS/nightly.n2.pub"; chmod 644 "$KEYS/nightly.n2.pub"
cp "$KEYS/nightly.n2.pub" "$T/n2.pub"
relist
sign "$T/n-rotated" nightly 100 n2
verify nightly "$T/n-rotated"
is  "a second nightly key (a rotation) is accepted" "ok:0" "$(field status):$rc"
is  "and named" "nightly.n2" "$(field key)"
rm -f "$KEYS/nightly.n2.pub"; relist
verify nightly "$T/n-rotated"
is  "removing the old key stops trusting it" "bad_signature:20" "$(field status):$rc"
# A key file nobody vouched for (not on the build's list) is ignored, whoever owns it.
cp "$T/n2.pub" "$KEYS/nightly.n2.pub"
verify nightly "$T/n-rotated"
is  "a key that is not on the installed build's list is not trusted" "bad_signature:20" "$(field status):$rc"
rm -f "$KEYS/nightly.n2.pub"
if [ "$(id -u)" != 0 ]; then
  # Outside the selftest the uid that counts is root's; nobody sets it from the environment.
  out="$(env -u YANTRIK_SELFTEST "$VERIFY" verify --channel nightly --keys "$KEYS" --state "$STATE" "$T/n100")"
  is "the ownership check cannot be switched off from the environment" "keys_untrusted" "$(field status)"
fi

# ── 7. The scripts that make and use keys ───────────────────────────────────────────────
hasnt "new-key.sh prints no private key" "PRIVATE" "$(cat "$T/n1.out")"
has   "it prints the public key" "BEGIN PUBLIC KEY" "$(cat "$T/n1.out")"
is    "the private key is mode 0600" "600" "$(stat -c %a "$T/n1.key")"
out="$("$NEWKEY" nightly n1 "$T/n1.key" 2>&1)"; rc=$?
is    "new-key.sh will not overwrite a key" "1" "$rc"
KEYVAL="$(cat "$T/n1.key")"
out="$(UPDATE_SIGNING_KEY_NIGHTLY="$KEYVAL" "$SIGN" --channel nightly --version v1 --git abc1234 \
  --artifact a.tar.zst --sha256 "$SHA" --key-env UPDATE_SIGNING_KEY_NIGHTLY --out "$T/fromenv" 2>&1)"; rc=$?
is    "sign-manifest signs with a key from the environment" "0" "$rc"
hasnt "and never echoes it" "$(sed -n 2p "$T/n1.key")" "$out"
verify nightly "$T/fromenv" --now "$(date +%s)"
is    "what it signed verifies" "ok:0" "$(field status):$rc"
out="$(env -u UPDATE_SIGNING_KEY_NIGHTLY "$SIGN" --channel nightly --version v1 --git abc1234 \
  --artifact a.tar.zst --sha256 "$SHA" --key-env UPDATE_SIGNING_KEY_NIGHTLY --out "$T/nokey" 2>&1)"; rc=$?
is    "an absent secret is a failure, not an unsigned file" "1:no" "$rc:$([ -e "$T/nokey" ] && echo yes || echo no)"
out="$(UPDATE_SIGNING_KEY_NIGHTLY="not a key" "$SIGN" --channel nightly --version v1 --git abc1234 \
  --artifact a.tar.zst --sha256 "$SHA" --key-env UPDATE_SIGNING_KEY_NIGHTLY --out "$T/badkey" 2>&1)"; rc=$?
is    "a value that is not a key is a failure" "1:no" "$rc:$([ -e "$T/badkey" ] && echo yes || echo no)"
openssl genrsa -out "$T/rsa.key" 2048 2>/dev/null
out="$("$SIGN" --channel nightly --version v1 --git abc1234 --artifact a.tar.zst --sha256 "$SHA" \
  --key-file "$T/rsa.key" --out "$T/rsa.out" 2>&1)"; rc=$?
is    "only ed25519 keys sign" "1" "$rc"

# ── 7b. The trusted key set only moves forward (#537 review) ────────────────────────────
echo "  rotation, rollback, revocation, planted keys"
extract() { sed -n "/^$1() {/,/^}/p" "$UPDATE"; }
RB="$T/restore.sh"; { extract restore_backup; extract is_updater_file; extract seal_share; } > "$RB"
rollback_in() { # <prefix> <backup> <tree is root's: 0|1>: restore_backup as the updater runs it
  ( PREFIX="$1"; BIN_DIR="$1/bin"; SHARE_DIR="$1/share"; UPDATE_KEYS_DIR="$1/share/update-keys"; BUILD_FILE="$1/BUILD"
    TREE_ROOT="$3"
    warn() { :; }; info() { :; }; swap_memory() { :; }; stamp_version_file() { :; }; installed_version() { :; }
    tree_is_roots() { [ "$TREE_ROOT" = 1 ]; }
    # shellcheck disable=SC1090
    . "$RB"; restore_backup "$2" ) > /dev/null 2>&1
}
P="$T/rot"; mkdir -p "$P/bin" "$P/share/update-keys"; chmod 755 "$P" "$P/share" "$P/share/update-keys"
cp "$VERIFY" "$P/bin/"; echo "tool" > "$P/bin/other"; chmod 755 "$P/bin"
KEYS="$P/share/update-keys"; STATE="$P/update-state"; LIST="$P/bin/yantrik-root-files.sha256"
# Build N-1: trusts n1.
cp "$T/n1.pub" "$KEYS/nightly.n1.pub"; chmod 644 "$KEYS"/*; relist
verify nightly "$T/n100"
is  "build N-1: its own key works" "ok:0" "$(field status):$rc"
mkdir -p "$T/bk"; cp -a "$P/bin" "$T/bk/bin"; cp -a "$P/share" "$T/bk/share"; chmod -R go-w "$T/bk"
# Build N rotates n1 out: n2 in, n1 revoked, and apply merges the revocation before installing.
FP1="$("$VERIFY" fingerprint "$T/n1.pub")"
printf '# rotated out
%s nightly.n1 leaked
' "$FP1" > "$T/bundle-revoked"
out="$("$VERIFY" revoke-merge --state "$STATE" "$T/bundle-revoked")"
has "applying a bundle records its revocations" "1 new" "$out"
rm -f "$KEYS/nightly.n1.pub"; cp "$T/n2.pub" "$KEYS/nightly.n2.pub"; cp "$T/bundle-revoked" "$KEYS/revoked.sha256"; chmod 644 "$KEYS"/*; relist
verify nightly "$T/n-rotated"
is  "build N: a normal update, signed by the new key, works" "ok:0" "$(field status):$rc"
verify nightly "$T/n100"
is  "build N: the revoked key is refused" "bad_signature:20" "$(field status):$rc"
# Rollback to N-1 (a failed install, a manual rollback or a downgrade restores its share/).
rollback_in "$P" "$T/bk" 0; rbrc=$?
is  "the rollback itself goes ahead" "0" "$rbrc"
is  "but the old key does not come back with it" "no" "$([ -e "$KEYS/nightly.n1.pub" ] && echo yes || echo no)"
is  "and the current key stays" "yes" "$([ -e "$KEYS/nightly.n2.pub" ] && echo yes || echo no)"
verify nightly "$T/n100"
is  "after the rollback the revoked key is still refused" "bad_signature:20" "$(field status):$rc"
verify nightly "$T/n-rotated"
is  "and the current one still works (the updater can still update)" "ok:0" "$(field status):$rc"
# Even a .pub put back by hand, on a list that names it (an older bundle's), is dead: revoked.
cp "$T/n1.pub" "$KEYS/nightly.n1.pub"; chmod 644 "$KEYS/nightly.n1.pub"; rm -f "$KEYS/revoked.sha256"; relist
verify nightly "$T/n100"
is  "a revoked key restored AND listed is still refused" "bad_signature:20" "$(field status):$rc"
"$VERIFY" revoke-merge --state "$STATE" "$T/empty" > /dev/null
printf '%s
' "$(printf 'c%.0s' $(seq 1 64))" > "$T/other-revoked"
"$VERIFY" revoke-merge --state "$STATE" "$T/other-revoked" > /dev/null
has "the revocation record only grows" "$FP1" "$(cat "$STATE/revoked-keys")"
is  "and holds the newer line too" "2" "$(wc -l < "$STATE/revoked-keys" | tr -d ' ')"
rm -f "$KEYS/nightly.n1.pub"; relist
# A key planted in a backup's share/.
rm -rf "$T/bk2"; cp -a "$T/bk" "$T/bk2"; chmod -R u+w "$T/bk2"
mkdir -p "$T/bk2/share/update-keys"; openssl pkey -in "$T/rogue.key" -pubout -out "$T/bk2/share/update-keys/release.planted.pub"
[ "$(id -u)" != 0 ] || chown -R 65534 "$T/bk2"
rollback_in "$P" "$T/bk2" 1; rbrc=$?
is  "a backup whose share/ is not root's alone is refused" "1" "$rbrc"
is  "and nothing was restored from it" "no" "$([ -e "$KEYS/release.planted.pub" ] && echo yes || echo no)"
rollback_in "$P" "$T/bk2" 0 || true
is  "even where ownership cannot be judged, a planted key is not copied in" "no" "$([ -e "$KEYS/release.planted.pub" ] && echo yes || echo no)"
sign "$T/planted-stable" stable 100 rogue
verify stable "$T/planted-stable"
is  "a manifest signed by the planted key is refused" "20" "$rc"
cp "$T/bk2/share/update-keys/release.planted.pub" "$KEYS/"; chmod 644 "$KEYS/release.planted.pub"
verify stable "$T/planted-stable"
is  "a planted key that is in share/ but not on the list is refused" "20" "$rc"
rm -f "$KEYS/release.planted.pub"
verify nightly "$T/n-rotated"
is  "the machine still updates normally afterwards" "ok:0" "$(field status):$rc"
KEYS="$PFX/share/update-keys"; STATE="$PFX/update-state"; LIST="$PFX/bin/yantrik-root-files.sha256"

# ── 7c. A machine that never applied anything, then downgraded ───────────────────────────
echo "  fresh machine on the rotating build, then downgraded"
fresh() { # <dir>: a prefix on the rotating build N: n2 trusted, n1 revoked, and no update-state at all
  rm -rf "$1"; mkdir -p "$1/bin" "$1/share/update-keys"; chmod 755 "$1" "$1/bin" "$1/share" "$1/share/update-keys"
  cp "$T/n2.pub" "$1/share/update-keys/nightly.n2.pub"; cp "$T/bundle-revoked" "$1/share/update-keys/revoked.sha256"
  chmod 644 "$1/share/update-keys"/*
  KEYS="$1/share/update-keys"; STATE="$1/update-state"; LIST="$1/bin/yantrik-root-files.sha256"; relist
}
downgrade() { # <dir>: the older build S replaces share/ and the list; it lists n1 and revokes nothing
  rm -rf "$1/share/update-keys"; mkdir "$1/share/update-keys"; chmod 755 "$1/share/update-keys"
  cp "$T/n1.pub" "$1/share/update-keys/nightly.n1.pub"; : > "$1/share/update-keys/revoked.sha256"
  chmod 644 "$1/share/update-keys"/*; relist
}
fresh "$T/img1"
verify nightly "$T/n100"
is  "imaged build N, no state: n1 is refused" "bad_signature:20" "$(field status):$rc"
is  "and there is no record yet" "no" "$([ -e "$STATE/revoked-keys" ] && echo yes || echo no)"
verify nightly "$T/n-rotated" --commit
is  "a normal update (commit) works" "ok:0" "$(field status):$rc"
has "and it records what the installed build revokes" "$FP1" "$(cat "$STATE/revoked-keys")"
downgrade "$T/img1"
verify nightly "$T/n100"
is  "downgraded to the older build: n1 is still refused" "no_keys:20" "$(field status):$rc"
# The updater's own step, for a machine that has not run verify --commit: merge the INSTALLED list.
fresh "$T/img2"
"$VERIFY" revoke-merge --state "$STATE" "$KEYS/revoked.sha256" > /dev/null
downgrade "$T/img2"
verify nightly "$T/n100"
is  "apply merging the installed list first gives the same" "no_keys:20" "$(field status):$rc"
# Without either step the older build would have trusted it again: the gap this closes.
fresh "$T/img3"; downgrade "$T/img3"
verify nightly "$T/n100"
is  "(control: with no record at all the older build does trust n1)" "ok:0" "$(field status):$rc"
# The lock-out guard apply runs: the bundle's own keys and list against the manifest just accepted.
fresh "$T/img4"; "$VERIFY" revoke-merge --state "$STATE" "$KEYS/revoked.sha256" > /dev/null
mkdir -p "$T/bundleS/bin" "$T/bundleS/share/update-keys"; chmod 755 "$T/bundleS" "$T/bundleS/bin" "$T/bundleS/share" "$T/bundleS/share/update-keys"
cp "$T/n1.pub" "$T/bundleS/share/update-keys/nightly.n1.pub"; chmod 644 "$T/bundleS/share/update-keys/nightly.n1.pub"
(cd "$T/bundleS/share" && sha256sum update-keys/*) > "$T/bundleS/bin/yantrik-root-files.sha256"; chmod 644 "$T/bundleS/bin/yantrik-root-files.sha256"
out="$("$VERIFY" verify --channel nightly --source releases.example --keys "$T/bundleS/share/update-keys" --state "$STATE" --listed "$T/bundleS/bin/yantrik-root-files.sha256" --now "$((NOW + 86400))" "$T/n-rotated")"
is  "a bundle whose keys cannot verify the current manifest is caught before install" "no_keys" "$(printf '%s\n' "$out" | sed -n 's/^status=//p')"
has "and the sentence says what to do" "install a current build from a fresh image" "$out"
# Revocation lines that revoke nothing are errors, not silence.
for bad in "$(printf 'A%.0s' $(seq 1 64))" " $(printf 'a%.0s' $(seq 1 64))" "$(printf 'a%.0s' $(seq 1 63))"; do
  printf '%s\n' "$bad" > "$T/badrev"
  out="$("$VERIFY" revoke-merge --state "$T/state-bad" "$T/badrev" 2>&1)"; rc=$?
  is "a malformed revocation line is refused ($(printf '%s' "$bad" | cut -c1-4)...)" "1" "$rc"
done
fresh "$T/img5"; printf '%s\n' "$(printf 'A%.0s' $(seq 1 64))" >> "$KEYS/revoked.sha256"
verify nightly "$T/n-rotated"
is  "a damaged list in the installed build is a refusal, not an ignored line" "revocations_malformed:20" "$(field status):$rc"
KEYS="$PFX/share/update-keys"; STATE="$PFX/update-state"; LIST="$PFX/bin/yantrik-root-files.sha256"

# ── 8. The real updater, against a real (local) web server ──────────────────────────────
echo "  the updater against a server"
WWW="$T/www"; mkdir -p "$WWW/nightly"
PORT="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
python3 -I -m http.server "$PORT" --bind 127.0.0.1 --directory "$WWW" >/dev/null 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 50); do python3 -I -c "import socket; socket.create_connection(('127.0.0.1', $PORT), 1)" 2>/dev/null && break; sleep 0.1; done
printf 'CHANNEL=nightly\nHOST=127.0.0.1:%s\nSCHEME=http\n' "$PORT" > "$PFX/update.conf"
printf 'git=0000000\nversion=v0.0.1\n' > "$PFX/BUILD"
cp "$VERIFY" "$PFX/bin/"; cp "$UPDATE" "$PFX/bin/"
chmod 755 "$PFX/bin/yantrik-update" "$PFX/bin/yantrik-update-verify"
upd() { out="$(env YANTRIK_PREFIX="$PFX" "$PFX/bin/yantrik-update" check --porcelain 2>&1)"; rc=$?; }

# A fresh signed manifest (the real clock: the updater reads it).
"$SIGN" --channel nightly --version v1.2.3 --git abc1234 --artifact yantrik-os-v1.2.3-linux-amd64.tar.zst \
  --sha256 "$SHA" --key-file "$T/n1.key" --out "$WWW/nightly/release.signed" >/dev/null
printf '{"channels":{"nightly":{"version":"v1.2.3","git":"abc1234"}}}' > "$WWW/manifest.json"
upd
is  "a signed channel is an available update" "update_available:10" "$(field result):$rc"
is  "the version shown comes from the signed manifest" "abc1234" "$(field channel_git)"
is  "check records nothing (it runs as the person)" "no" "$([ -e "$PFX/update-state/nightly.serial" ] && echo yes || echo no)"

# The server only has the old unsigned manifest: the migration window is closed on this side.
rm "$WWW/nightly/release.signed"
upd
is  "an unsigned channel is refused, not believed" "unverified:14" "$(field result):$rc"
has "with the sentence" "is not signed by Yantrik, so nothing was installed" "$(field reason)"
is  "and no build is shown as available" "" "$(field channel_git)"

# Someone swaps in a manifest signed by a key nobody trusts.
"$SIGN" --channel nightly --version v6.6.6 --git evil666 --artifact x.tar.zst --sha256 "$SHA" \
  --key-file "$T/rogue.key" --out "$WWW/nightly/release.signed" >/dev/null
upd
is  "a manifest signed by an unknown key is refused" "unverified:14" "$(field result):$rc"

# The nightly key signs a stable manifest and the server serves it as the nightly one.
"$SIGN" --channel stable --version v6.6.6 --git evil666 --artifact x.tar.zst --sha256 "$SHA" \
  --key-file "$T/n1.key" --out "$WWW/nightly/release.signed" >/dev/null
upd
is  "a manifest for another channel is refused" "unverified:14" "$(field result):$rc"

# Nothing published on the channel at all is still just "not published".
rm "$WWW/nightly/release.signed"; printf '{"channels":{}}' > "$WWW/manifest.json"
upd
is  "an unpublished channel is still reported as such" "not_published:11" "$(field result):$rc"
kill "$SERVER_PID" 2>/dev/null; SERVER_PID=""
upd
is  "and an unreachable server as unreachable" "unreachable:12" "$(field result):$rc"
has "the updater's own docs mention the new exit" "14 update information not" "$(env YANTRIK_PREFIX="$PFX" "$PFX/bin/yantrik-update" help)"

echo
if [ "$fails" = 0 ]; then echo "all passed"; exit 0; fi
echo "$fails failed"; exit 1
