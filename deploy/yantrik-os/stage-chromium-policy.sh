#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════════════════
# stage-chromium-policy.sh — Chromium's no-phone-home policy, put into a root (#1175)
# ═══════════════════════════════════════════════════════════════════════════════════════
#
#   sh stage-chromium-policy.sh ROOT POLICY.json     install into ROOT/etc/chromium/policies/managed
#   sh stage-chromium-policy.sh selftest
#
# The one install step. The ISO build, the cloud-image build and `yantrik-update reconcile` all
# call this, so the file's place, name and mode are decided once: root-owned 0644 at
# /etc/chromium/policies/managed/yantrik-no-phone-home.json (Debian's chromium reads that folder).
# ROOT is / on an installed machine and the rootfs in a build. Only that one file is written: a
# person's own managed policies beside it are never read, changed or removed.
set -eu

NAME=yantrik-no-phone-home.json
DIR=etc/chromium/policies/managed

# A folder this script may write into: a real folder (not a link), owned by whoever runs it (root,
# on every machine and in every build; the caller's own account only in a non-root test run), and
# not writable by group or others. Anything else is somebody's own arrangement and is left alone.
folder_ok() {
    [ ! -L "$1" ] && [ -d "$1" ] || return 1
    [ "$(stat -c %u "$1")" = "$(id -u)" ] || return 1
    [ -z "$(find "$1" -maxdepth 0 -perm /022)" ]
}

refuse() { echo "stage-chromium-policy: refusing: $*" >&2; return 3; }

install_policy() {
    root="$1" src="$2"
    [ -f "$src" ] || { echo "stage-chromium-policy: no $src" >&2; return 1; }
    python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert isinstance(d,dict) and d' "$src" \
        || { echo "stage-chromium-policy: $src is not a JSON object" >&2; return 1; }
    root="${root%/}"
    # Each existing component of the path must be a plain folder of ours; only missing ones are made,
    # 0755. An existing folder is never re-owned, re-moded or followed through a link.
    p="$root"
    for c in etc chromium policies managed; do
        p="$p/$c"
        if [ -e "$p" ] || [ -L "$p" ]; then
            folder_ok "$p" || refuse "$p is a link, not a folder, not owned by $(id -un), or writable by others" || return $?
        else
            mkdir -m 0755 "$p"
        fi
    done
    dest="$root/$DIR/$NAME"
    if [ -e "$dest" ] || [ -L "$dest" ]; then
        { [ -f "$dest" ] && [ ! -L "$dest" ]; } || refuse "$dest exists and is not a regular file" || return $?
    fi
    if [ "$(id -u)" = 0 ]; then
        install -m 0644 -o root -g root "$src" "$dest"
    else
        install -m 0644 "$src" "$dest"
    fi
}

selftest() {
    t="$(mktemp -d)"; trap 'rm -rf "$t"' EXIT
    here="$(cd "$(dirname "$0")" && pwd)"
    fail() { echo "stage-chromium-policy selftest: $*" >&2; exit 1; }
    mkdir -p "$t/root/$DIR"
    printf '{"HomepageLocation":"https://example.org"}\n' > "$t/root/$DIR/mine.json"
    cp "$t/root/$DIR/mine.json" "$t/mine.before"
    install_policy "$t/root" "$here/chromium-managed-policy.json"
    [ -f "$t/root/$DIR/$NAME" ] || fail "the policy was not installed"
    [ "$(stat -c %a "$t/root/$DIR/$NAME")" = 644 ] || fail "mode is not 0644"
    [ "$(id -u)" != 0 ] || [ "$(stat -c %U "$t/root/$DIR/$NAME")" = root ] || fail "not root-owned"
    cmp -s "$t/mine.before" "$t/root/$DIR/mine.json" || fail "a person's own policy was changed"
    install_policy "$t/root" "$here/chromium-managed-policy.json"    # again: same result
    [ "$(ls "$t/root/$DIR" | wc -l)" = 2 ] || fail "a second run left something else behind"
    printf 'not json' > "$t/bad.json"
    if install_policy "$t/root2" "$t/bad.json" 2>/dev/null; then fail "invalid JSON was installed"; fi
    [ ! -e "$t/root2/$DIR/$NAME" ] || fail "invalid JSON left a file"
    # The shipped policy: valid, and no key outside the ones checked against Chromium 153.
    python3 -I - "$here/chromium-managed-policy.json" <<'PY' || fail "the shipped policy has a key not checked against Chromium"
import json, sys
known = {"GenAILocalFoundationalModelSettings", "BackgroundModeEnabled", "MetricsReportingEnabled",
         "UrlKeyedAnonymizedDataCollectionEnabled", "SyncDisabled"}
extra = set(json.load(open(sys.argv[1]))) - known
sys.exit("unchecked keys: %s" % sorted(extra) if extra else 0)
PY
    # Folders that are not ours are refused, and nothing is written through them.
    mkdir -p "$t/l/etc/chromium/policies" "$t/l/elsewhere"
    ln -s "$t/l/elsewhere" "$t/l/etc/chromium/policies/managed"
    if install_policy "$t/l" "$here/chromium-managed-policy.json" 2>/dev/null; then fail "a symlinked folder was written through"; fi
    [ -z "$(ls "$t/l/elsewhere")" ] || fail "the policy was written through a symlink"
    mkdir -p "$t/l2/etc/chromium/policies/managed"; chmod 0777 "$t/l2/etc/chromium/policies/managed"
    if install_policy "$t/l2" "$here/chromium-managed-policy.json" 2>/dev/null; then fail "a world-writable folder was used"; fi
    [ "$(stat -c %a "$t/l2/etc/chromium/policies/managed")" = 777 ] || fail "a folder that is not ours was re-moded"
    if [ "$(id -u)" = 0 ]; then
        mkdir -p "$t/l3/etc/chromium/policies/managed"; chown 1000:1000 "$t/l3/etc/chromium/policies/managed"
        if install_policy "$t/l3" "$here/chromium-managed-policy.json" 2>/dev/null; then fail "a folder of another account was used"; fi
        [ "$(stat -c %u "$t/l3/etc/chromium/policies/managed")" = 1000 ] || fail "a folder of another account was re-owned"
    else
        echo "  skip: a folder owned by another account (needs root to make one)"
    fi
    mkdir -p "$t/l4/etc/chromium/policies/managed"; ln -s "$t/mine.before" "$t/l4/etc/chromium/policies/managed/$NAME"
    if install_policy "$t/l4" "$here/chromium-managed-policy.json" 2>/dev/null; then fail "a symlink in the file's place was used"; fi
    cmp -s "$t/mine.before" "$t/root/$DIR/mine.json" || fail "a person's file changed"
    mkdir -p "$t/l5"; install_policy "$t/l5" "$here/chromium-managed-policy.json"
    [ "$(stat -c %a "$t/l5/etc" "$t/l5/$DIR")" = "755
755" ] || fail "made folders are not 0755"
    # Every path that makes a machine calls this script (never a copy of its steps).
    for f in build-debian-iso.sh build-image.sh; do
        [ ! -f "$here/$f" ] || grep -qF 'stage-chromium-policy.sh' "$here/$f" || fail "$f does not install the policy"
    done
    [ ! -f "$here/yantrik-update" ] || { grep -qF 'stage-chromium-policy.sh' "$here/yantrik-update" \
        && grep -qE '^[[:space:]]*reconcile_chromium_policy[[:space:]]*(\|\||$)' "$here/yantrik-update"; } \
        || fail "yantrik-update reconcile does not install the policy"
    [ ! -f "$here/build-release.sh" ] || { grep -qF 'stage-chromium-policy.sh' "$here/build-release.sh" \
        && grep -qE 'sha256sum .* chromium/\*' "$here/build-release.sh"; } || fail "the release bundle does not ship and hash it"
    echo "stage-chromium-policy selftest: all passed"
}

case "${1:-}" in
    selftest) selftest ;;
    ""|-h|--help) echo "usage: stage-chromium-policy.sh ROOT POLICY.json | selftest" >&2; exit 2 ;;
    *) [ $# -eq 2 ] || { echo "usage: stage-chromium-policy.sh ROOT POLICY.json" >&2; exit 2; }
       install_policy "$1" "$2" ;;
esac
