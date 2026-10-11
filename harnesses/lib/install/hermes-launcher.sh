# Hermes's launcher, pointed at the Python that stays (#712). Sourced, not run.
#
# Hermes's installer fetches its uv and Python into a temporary directory, keeps the Python by
# copying it to ~/.hermes/tools, and then writes the launcher that `hermes` and the gateway unit
# both run (~/.hermes/hermes-agent/.hermes/bin/hermes) with the temporary copy's path in it:
#
#   exec /tmp/tmp.XXXX/tools/python-3.14.7+20260901-linux-x64/bin/python3 -I -c '…'
#
# It worked until /tmp was next cleaned. Then the hermes command died and the gateway would not
# restart, on a machine where the same Python sat untouched in ~/.hermes/tools.
#
# One function, used by the installer (harnesses/lib/install/hermes.sh) straight after Hermes's
# own installer runs, and by `yantrik-update reconcile` for machines installed before this, so
# both repair the same way. POSIX sh, and nothing from common.sh: the updater sources it too.

# hermes_repoint_launcher HOME [keep]
#
# Every file in HOME's Hermes launcher directory that names an interpreter under
# /tmp/*/tools/NAME/bin/ is rewritten to name the same interpreter under HOME/.hermes/tools/NAME/bin/
# (that exact file, else that directory's python3, else its python3.N). With `keep`, the file as
# it was is kept beside it as FILE.before-712, not executable. Fails, saying why, when there
# is no interpreter to point at or a /tmp path survives the rewrite; the launcher is then left as
# it was. A launcher that names nothing under /tmp is not touched.
hermes_repoint_launcher() {
    hl_home=$1 hl_keep=${2:-}
    # A path that starts at /tmp: /var/tmp/x, or a /tmp/ inside some other path, is not one.
    hl_at="(^|[^[:alnum:]_./-])/tmp/"
    hl_bin="$hl_home/.hermes/hermes-agent/.hermes/bin"
    hl_tools="$hl_home/.hermes/tools"
    [ -d "$hl_bin" ] || return 0
    for hl_f in "$hl_bin"/*; do
        [ -f "$hl_f" ] && [ ! -L "$hl_f" ] || continue
        # A copy this function kept: still names /tmp, and is meant to.
        case "$hl_f" in *.before-712) continue ;; esac
        grep -qE "$hl_at" "$hl_f" || continue

        # Each distinct interpreter path, then a sed expression for each. Whitespace, quotes and
        # glob characters end a path, so the unquoted loop below splits only between paths.
        set --
        for hl_old in $(grep -oE "${hl_at}[^][[:space:]'\"*?]*/tools/[^]/[:space:]'\"*?[]*/bin/[^]/[:space:]'\"*?[]*" "$hl_f" | sort -u); do
            hl_old="/tmp/${hl_old#*/tmp/}"
            hl_name=${hl_old%/bin/*}; hl_name=${hl_name##*/}
            hl_dir="$hl_tools/$hl_name/bin"
            hl_new="$hl_dir/${hl_old##*/}"
            [ -x "$hl_new" ] || hl_new="$hl_dir/python3"
            if [ ! -x "$hl_new" ]; then
                for hl_c in "$hl_dir"/python3.[0-9]*; do
                    [ -x "$hl_c" ] && { hl_new=$hl_c; break; }
                done
            fi
            if [ ! -x "$hl_new" ]; then
                printf '%s names %s, and there is no %s/%s to point it at\n' \
                    "$hl_f" "$hl_old" "$hl_dir" "${hl_old##*/}" >&2
                return 1
            fi
            set -- "$@" -e "s|$(printf '%s' "$hl_old" | sed 's/[].[*^$\\|]/\\&/g')|$(printf '%s' "$hl_new" | sed 's/[\\|&]/\\&/g')|g"
        done
        if [ $# -eq 0 ]; then
            printf '%s names something under /tmp that is not an interpreter in tools/; it was left as it is\n' "$hl_f" >&2
            return 1
        fi

        # Written beside it with its mode, checked, then renamed over it: a launcher is never
        # half-written, and one that would still name /tmp is never put in place.
        cp -p "$hl_f" "$hl_f.repoint" && sed "$@" "$hl_f" > "$hl_f.repoint" \
            || { rm -f "$hl_f.repoint"; printf 'could not rewrite %s\n' "$hl_f" >&2; return 1; }
        if grep -qE "$hl_at" "$hl_f.repoint"; then
            rm -f "$hl_f.repoint"
            printf '%s still names /tmp after repointing; it was left as it is\n' "$hl_f" >&2
            return 1
        fi
        if [ -n "$hl_keep" ]; then
            cp -p "$hl_f" "$hl_f.before-712" && chmod a-x "$hl_f.before-712" \
                || { rm -f "$hl_f.repoint"; printf 'could not keep a copy of %s\n' "$hl_f" >&2; return 1; }
        fi
        mv -f "$hl_f.repoint" "$hl_f" || { rm -f "$hl_f.repoint"; return 1; }
        printf '%s now runs the Python in %s\n' "$hl_f" "$hl_tools"
    done
    return 0
}
