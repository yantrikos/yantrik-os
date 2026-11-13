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

install_policy() {
    root="$1" src="$2"
    [ -f "$src" ] || { echo "stage-chromium-policy: no $src" >&2; return 1; }
    python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert isinstance(d,dict) and d' "$src" \
        || { echo "stage-chromium-policy: $src is not a JSON object" >&2; return 1; }
    root="${root%/}"
    install -d -m 0755 -o root -g root "$root/$DIR" 2>/dev/null || install -d -m 0755 "$root/$DIR"
    if [ "$(id -u)" = 0 ]; then
        install -m 0644 -o root -g root "$src" "$root/$DIR/$NAME"
    else
        install -m 0644 "$src" "$root/$DIR/$NAME"
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
         "UrlKeyedAnonymizedDataCollectionEnabled", "SyncDisabled", "TranslateEnabled", "ComponentUpdatesEnabled"}
extra = set(json.load(open(sys.argv[1]))) - known
sys.exit("unchecked keys: %s" % sorted(extra) if extra else 0)
PY
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
