#!/bin/bash
# The check helpers shared by the harness isolation selftests. Sourced, not run.
fails=0
is() { # <name> <expected> <actual>
  if [ "$2" = "$3" ]; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n       want: %s\n       got:  %s\n' "$1" "$2" "$3"; fails=$((fails + 1)); fi
}
has() { # <name> <needle> <haystack>
  case "$3" in *"$2"*) printf '  ok   %s\n' "$1" ;; *) printf '  FAIL %s\n       missing: %s\n' "$1" "$2"; fails=$((fails + 1)) ;; esac
}
lacks() { # <name> <needle> <haystack>
  case "$3" in *"$2"*) printf '  FAIL %s\n       found: %s\n' "$1" "$2"; fails=$((fails + 1)) ;; *) printf '  ok   %s\n' "$1" ;; esac
}
yes() { if "${@:2}" >/dev/null 2>&1; then printf '  ok   %s\n' "$1"; else printf '  FAIL %s\n' "$1"; fails=$((fails + 1)); fi; }
no() { if "${@:2}" >/dev/null 2>&1; then printf '  FAIL %s\n' "$1"; fails=$((fails + 1)); else printf '  ok   %s\n' "$1"; fi; }
# shellcheck disable=SC2154  # $here is the sourcing selftest's own directory
scope_supported() { # does this kernel have Landlock ABI 6, which the abstract-socket scope needs?
  python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); import yantrik_landlock as l; sys.exit(0 if l.abi() >= l.MIN_ABI else 1)' "$here" 2>/dev/null
}
