#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════
# service-bins.sh — the binaries the workspace's services build, asked of the manifests
# ═══════════════════════════════════════════════════════════════════════════════
#
# Prints one binary name per line. Exits non-zero if it cannot read the list, because a
# deploy that silently gets no services reports success and leaves the machine without
# them — the copy loops that consume this skip a binary that is not there, and the shell
# registers its services whether or not the binaries ever arrived.
#
# A service is a workspace member under services/ that carries a yantrik.toml manifest,
# and its binary is the [service] `binary = "…"` in that manifest — the same manifests
# start_services (crates/yantrik-ui/src/main.rs) scans on an installed machine, so the
# list that ships is derived the way the shell itself finds services. A member without a
# manifest is a helper the shell never registers, and is skipped rather than failed over —
# selftest check 5 is what keeps that skip from hiding a service that forgot its manifest.
#
# That is the rule deploy.sh, its BUILD_ALL branch and scripts/publish-components.sh
# were each expressing by hand, and the copies disagreed with each other: BUILD_ALL, the
# branch meant to build more, built two services fewer than the default, and a channel
# published with publish-components.sh carried a shell that registered `a11y` and
# `perception` with no binaries behind them. Nobody writes the list down now; a new
# service under services/ is built, deployed and published by existing code.
#
# The shelf still wins: callers drop what shelved-bins.sh names, the same way they
# always did.
#
#   deploy/yantrik-os/service-bins.sh                 # one name per line
#   SERVICE_BINS="$(deploy/yantrik-os/service-bins.sh)"
#   deploy/yantrik-os/service-bins.sh selftest        # offline checks against the tree
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
WORKSPACE="$PROJECT_ROOT/Cargo.toml"
MAIN_RS="$PROJECT_ROOT/crates/yantrik-ui/src/main.rs"

# Crate directory names under services/ that are deliberately not services — a crate that
# lives there for cargo's sake and carries no yantrik.toml because the shell has nothing to
# register. Every entry needs its reason written next to it, and selftest check 5 fails an
# entry whose directory is gone or which grew a manifest after all.
NOT_SHIPPED=""

# The `binary = "…"` value in a manifest's [service] section. A section runs from its
# header to the next header; only a line at column zero is a header, so nothing indented
# is ever read as one.
service_binary_of() {
  awk '
    /^\[service\]/ { insvc = 1; next }
    /^\[/          { insvc = 0 }
    insvc && /^binary[ \t]*=[ \t]*"/ {
      sub(/^binary[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
    }
  ' "$1"
}

list_service_bins() {
  [ -f "$WORKSPACE" ] || {
    echo "service-bins.sh: cannot find $WORKSPACE — refusing to guess what the services are" >&2
    return 1
  }

  # The members = [ … ] array, keeping only the entries under services/. The array is the
  # list cargo itself builds from; a directory under services/ that is not a member is
  # not built, so it is not a service anything can ship.
  local members
  members="$(sed -n '/^members = \[/,/^\]/p' "$WORKSPACE" \
    | sed -n 's|^[[:space:]]*"\(services/[^"]\+\)".*|\1|p')"

  if [ -z "$members" ]; then
    echo "service-bins.sh: read no services/ members from $WORKSPACE." >&2
    echo "  Either the workspace has no services — in which case delete this guard, not" >&2
    echo "  the callers — or the members array changed shape and every packaging script" >&2
    echo "  is now shipping none of them. Failing rather than returning nothing." >&2
    return 1
  fi

  local bins="" member manifest cargo_toml name built
  for member in $members; do
    manifest="$PROJECT_ROOT/$member/yantrik.toml"
    # No manifest is not a service the shell scans and registers; helpers live under
    # services/ too, and the manifest is what tells them apart.
    [ -f "$manifest" ] || continue
    name="$(service_binary_of "$manifest")"
    if [ -z "$name" ]; then
      echo "service-bins.sh: read no binary = … from the [service] section of $manifest" >&2
      return 1
    fi
    # The manifest names the binary the shell looks for; the member's Cargo.toml names
    # the binary cargo builds. If the two ever disagree the build goes green, the copy
    # loop skips and the registration points at nothing — the silent absence this script
    # exists to stop — so the manifest is held against the [[bin]] targets, or against
    # the package name when the target is left to cargo.
    cargo_toml="$PROJECT_ROOT/$member/Cargo.toml"
    if [ ! -f "$cargo_toml" ]; then
      echo "service-bins.sh: $WORKSPACE lists member $member but $cargo_toml is not there" >&2
      return 1
    fi
    built="$(awk '
      /^\[\[bin\]\]/ { inbin = 1; next }
      /^\[/          { inbin = 0 }
      inbin && /^name[ \t]*=[ \t]*"/ {
        sub(/^name[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print
      }
    ' "$cargo_toml")"
    if [ -z "$built" ]; then
      built="$(awk '
        /^\[package\]/ { inpkg = 1; next }
        /^\[/          { inpkg = 0 }
        inpkg && /^name[ \t]*=[ \t]*"/ {
          sub(/^name[ \t]*=[ \t]*"/, ""); sub(/".*$/, ""); print; exit
        }
      ' "$cargo_toml")"
    fi
    case " $(echo "$built" | tr '\n' ' ') " in
      *" $name "*) ;;
      *)
        echo "service-bins.sh: $manifest names binary \"$name\", which $cargo_toml does not build." >&2
        echo "  It builds: $built" >&2
        return 1
        ;;
    esac
    bins="$bins$name
"
  done

  if [ -z "$bins" ]; then
    echo "service-bins.sh: derived no binaries from the services/ members of $WORKSPACE —" >&2
    echo "  none of them carries a yantrik.toml the shell could scan." >&2
    return 1
  fi

  printf '%s' "$bins"
}

# ── selftest ────────────────────────────────────────────────────────────────────────
#
# No builds, no machine, no network. It holds the two things that had more than one
# answer between them: the list the shell registers (the mgr.register fallback in
# start_services) against the list the packaging scripts are given here — a channel
# whose manifest lacks a binary the shell registers is the defect this script was
# written for — and the packaging scripts against this reader, so a service named by
# hand in one of them again fails here instead of silently not shipping. Since #705 it
# also holds every crate under services/ against the manifests: the reader skips a member
# without one, and that skip is how perception-journal sat built, tested and unstarted.
cmd_selftest() {
  local fails=0 out flat
  local c_grn c_red c_off
  c_grn="$(printf '\033[0;32m')"; c_red="$(printf '\033[0;31m')"; c_off="$(printf '\033[0m')"

  t_ok()   { printf '  %sok%s   %s\n' "$c_grn" "$c_off" "$1"; }
  t_fail() { printf '  %sFAIL%s %s\n' "$c_red" "$c_off" "$1"; fails=$((fails + 1)); }

  echo "service-bins.sh selftest"

  # 1. The reader reads the tree.
  if ! out="$(list_service_bins)"; then
    t_fail "the reader could not read the service list from the tree (its own message is above)"
    printf '%s%s selftest check(s) failed%s\n' "$c_red" "$fails" "$c_off"
    return 1
  fi
  t_ok "the reader derived $(echo "$out" | wc -l) service binaries from the workspace"
  flat="$(echo "$out" | tr '\n' ' ')"

  # 2. Derived again by another route: walk services/*/yantrik.toml instead of parsing
  #    the members array, and every manifest's binary must be in the answer. A parse
  #    that silently lost a member loses this check too.
  local manifest declared found_manifests=0
  for manifest in "$PROJECT_ROOT"/services/*/yantrik.toml; do
    [ -f "$manifest" ] || continue
    found_manifests=$((found_manifests + 1))
    declared="$(service_binary_of "$manifest")"
    case " $flat " in
      *" $declared "*) ;;
      *) t_fail "$manifest declares \"$declared\" and the reader did not list it — either its crate is not a workspace member (cargo will never build it) or the reader lost it" ;;
    esac
  done
  if [ "$found_manifests" -eq 0 ]; then
    t_fail "found no services/*/yantrik.toml manifests — the tree changed shape under this check"
  else
    t_ok "every one of the $found_manifests service manifests is in the reader's answer"
  fi

  # 3. Every binary the shell registers in start_services is one the packaging scripts
  #    are given. This is the shipped defect, held from the shell's side: a channel
  #    published without the binary behind a registration boots a desktop whose
  #    `yos describe` promises a service nothing can answer.
  [ -f "$MAIN_RS" ] || {
    t_fail "cannot find $MAIN_RS — the shell's registrations have nothing to be held against"
    printf '%s%s selftest check(s) failed%s\n' "$c_red" "$fails" "$c_off"
    return 1
  }
  local registered reg missing="" scripted
  registered="$(sed -n 's/.*mgr\.register([^,]*,[[:space:]]*"\([^"]*\)".*/\1/p' "$MAIN_RS")"
  # A service that is a program rather than a crate — yantrik-browser, Python on the surface SDK
  # — is not built by cargo; the release installs it from apps/ itself. Those installs are read
  # out of build-release.sh, the script that ships them, so this still writes no name down.
  scripted="$(sed -n 's|.*install -m 0755 "$PROJECT_ROOT/apps/[^"]*" "$ROOT/bin/\([^"]*\)".*|\1|p' \
    "$SCRIPT_DIR/build-release.sh" 2>/dev/null | paste -sd' ' -)"
  flat="$flat $scripted"
  if [ -z "$registered" ]; then
    t_fail "read no mgr.register(…) lines from $MAIN_RS — start_services changed shape"
  fi
  for reg in $registered; do
    case " $flat " in
      *" $reg "*) ;;
      *) missing="$missing $reg" ;;
    esac
  done
  if [ -n "$missing" ]; then
    t_fail "the shell registers$missing but the reader does not list them — packaging ships registrations with no binaries behind them"
  else
    t_ok "every binary start_services registers is in the list the packaging scripts get"
  fi

  # 4. And each packaging script asks the reader and writes no name down. A service
  #    named literally in a packaging script is a copy of the list, and the copies are
  #    what disagreed: BUILD_ALL built two services fewer than the default beside it.
  local rel text bin
  for rel in deploy.sh scripts/publish-components.sh; do
    text="$(cat "$PROJECT_ROOT/$rel")"
    case "$text" in
      *"service-bins.sh"*) t_ok "$rel asks service-bins.sh which services exist" ;;
      *) t_fail "$rel no longer asks service-bins.sh which services exist, so its list is hand-written again and the next service under services/ is silently not shipped" ;;
    esac
    for bin in $flat; do
      case "$text" in
        *"$bin"*) t_fail "$rel writes $bin down by hand — a name in the script is a copy that can go stale; ask deploy/yantrik-os/service-bins.sh instead" ;;
      esac
    done
  done

  # 5. Every crate under services/ ships or is exempt. The reader skips a member without a
  #    manifest — the rule that lets helpers live there — and that skip is how
  #    perception-journal shipped for a release: built by cargo, tested by CI, started by
  #    nothing. A directory under services/ with no yantrik.toml and no NOT_SHIPPED entry
  #    fails here, and so does a directory cargo never builds (not a services/… member of
  #    the workspace — no build, so nothing for any packaging script to find). The listing
  #    path above stays lenient; a helper may live under services/ without tripping a
  #    deploy. What fails is the silence, in selftest only.
  local members5 dir base exempt fails5=$fails
  members5="$(sed -n '/^members = \[/,/^\]/p' "$WORKSPACE" \
    | sed -n 's|^[[:space:]]*"\(services/[^"]\+\)".*|\1|p' | sed 's|^services/||' | paste -sd' ' -)"
  for dir in "$PROJECT_ROOT"/services/*/; do
    [ -d "$dir" ] || continue
    base="$(basename "$dir")"
    case " $NOT_SHIPPED " in
      *" $base "*) continue ;;
    esac
    if [ ! -f "$dir/yantrik.toml" ]; then
      t_fail "services/$base carries no yantrik.toml, so nothing ships or starts it — add the manifest, or name it in NOT_SHIPPED at the top of this script with a reason"
    fi
    case " $members5 " in
      *" $base "*) ;;
      *) t_fail "services/$base is not a services/… member of $WORKSPACE — cargo never builds it, so no packaging can ship it; add it to the workspace or move it out of services/" ;;
    esac
  done
  for exempt in $NOT_SHIPPED; do
    if [ ! -d "$PROJECT_ROOT/services/$exempt" ]; then
      t_fail "NOT_SHIPPED names \"$exempt\" and services/$exempt does not exist — the exemption is stale, delete it"
    elif [ -f "$PROJECT_ROOT/services/$exempt/yantrik.toml" ]; then
      t_fail "NOT_SHIPPED still exempts \"$exempt\" and services/$exempt/yantrik.toml is there — it is a service now, drop the exemption"
    fi
  done
  if [ "$fails" -eq "$fails5" ]; then
    t_ok "every crate under services/ carries a manifest or a NOT_SHIPPED reason"
  fi

  if [ "$fails" -gt 0 ]; then
    printf '%s%s selftest check(s) failed%s\n' "$c_red" "$fails" "$c_off"
    return 1
  fi
  printf '%sall selftest checks passed%s\n' "$c_grn" "$c_off"
}

case "${1:-}" in
  selftest) cmd_selftest ;;
  "")       list_service_bins ;;
  *)
    echo "usage: service-bins.sh [selftest]" >&2
    exit 2
    ;;
esac
