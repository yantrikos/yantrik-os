#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# shelved-bins.sh — the binaries this build does not ship, asked of the one place that decides
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# Prints one binary name per line, and nothing when the table is empty (it is, since Music
# and ySheets came off it in #727 and #728). Exits non-zero if it cannot find the table,
# because a packaging script that silently gets an empty list from a table it failed to read
# ships the shelved apps — which is the failure this file exists to stop.
#
# The shelf is SHELVED in crates/yantrik-ui/src/wire/dock.rs: each entry carries the app's
# names, the binary it would run, why it is not in this build, and what would bring it back.
# tests/app-lints/shelved.toml is the lint's view of the same decision and a test in dock.rs
# fails if the two stop agreeing; design/shelved-2026-09-20.md is the account.
#
# Five scripts used to carry their own copy of this list — build-release.sh, deploy.sh,
# install.sh, scripts/package-all.sh and scripts/publish-components.sh — and four of them
# still named the shelved apps, so `./deploy.sh` put Music and ySheets back on a machine that
# the shipped build refuses to open. A list written down in five places is a list that is
# wrong in four of them. Nobody writes it down now.
#
#   deploy/yantrik-os/shelved-bins.sh            # one name per line
#   SHELVED_BINS="$(deploy/yantrik-os/shelved-bins.sh | paste -sd' ' -)"
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DOCK="$PROJECT_ROOT/crates/yantrik-ui/src/wire/dock.rs"

[ -f "$DOCK" ] || {
  echo "shelved-bins.sh: cannot find $DOCK — refusing to guess what is shelved" >&2
  exit 1
}

# Only the VALUES inside the SHELVED table, never the struct's field declaration and never a
# test fixture further down the file: `pub binary:` is a type, `binary: "yantrik-tunes"` in a
# test is an example, and `binary: "…"` between `const SHELVED: &[Shelved] = &[` and its `];`
# is a decision. An empty table (`= &[];` on one line) is a decision too — nothing is shelved —
# and prints nothing. What still fails is a dock.rs with no SHELVED table in it, or one that
# never closes: that is the table changing shape, and every packaging script would then ship
# apps the launcher refuses to open.
BINS="$(awk '
  /^const SHELVED: &\[Shelved\] = &\[/ { found = 1; inside = 1 }
  inside && /^[ \t]+binary:[ \t]*"[^"]+"/ {
    match($0, /"[^"]+"/); print substr($0, RSTART + 1, RLENGTH - 2)
  }
  inside && /\];[ \t]*$/ { inside = 0; closed = 1 }
  END { if (!found || !closed) exit 3 }
' "$DOCK")" || {
  echo "shelved-bins.sh: found no SHELVED table in $DOCK (looked for a line starting" >&2
  echo "  'const SHELVED: &[Shelved] = &[' and the '];' that closes it)." >&2
  echo "  The table changed shape, and every packaging script would now ship apps the" >&2
  echo "  launcher refuses to open. Failing rather than returning nothing." >&2
  exit 1
}

[ -z "$BINS" ] || printf '%s\n' "$BINS"
