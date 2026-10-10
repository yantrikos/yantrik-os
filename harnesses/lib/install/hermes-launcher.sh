# Heal a Hermes launcher that still points at the installer's temporary Python.
#
# Hermes's own installer runs inside a `mktemp -d` directory and writes the path of the Python it
# unpacked there into `~/.hermes/hermes-agent/.hermes/bin/hermes`. That directory is removed when
# the installer's trap fires, so the launcher dies with `python3: not found` once /tmp is cleaned
# or the machine reboots. The same interpreter lives persistently at `~/.hermes/tools/...`, so the
# launcher can be repointed there. Sourced (never run) by hermes.sh and by yantrik-update, so the
# one implementation is shared and the two callers cannot drift apart.
#
# heal_hermes_launcher HOME_DIR
#
# For every regular text file in HOME_DIR/.hermes/hermes-agent/.hermes/bin/, find an interpreter
# path of the form /tmp/<one component>/tools/<rest> and map it to HOME_DIR/.hermes/tools/<rest>.
# Rewrite only when the old path does not exist and the mapped one is an executable file; otherwise
# leave the file alone and say so. Returns 0 when nothing is wrong, non-zero if a script still has
# a /tmp/<x>/tools/ path after the attempt.
# The callers source common.sh (hermes.sh) or define say themselves (yantrik-update); a test that
# sources only this file still needs a say to print through.
command -v say >/dev/null 2>&1 || say() { printf '%s\n' "$*"; }

heal_hermes_launcher() {
    home=$1
    bin="$home/.hermes/hermes-agent/.hermes/bin"
    tools="$home/.hermes/tools"
    [ -d "$bin" ] || return 0
    still_bad=0
    for f in "$bin"/*; do
        [ -f "$f" ] || continue
        case "$f" in *.before-tmp-fix) continue ;; esac
        # Only text scripts are launchers; a binary there is not ours to touch.
        grep -Iq . "$f" 2>/dev/null || continue
        # The interpreter path is /tmp/<one component>/tools/<rest>. Match that shape only, never
        # a bare /tmp mention, and never build a regex from the path itself.
        old=$(grep -o '/tmp/[^/ "]*/tools/[^ "]*' "$f" 2>/dev/null | head -n 1) || old=""
        [ -n "$old" ] || continue
        rest=${old#/tmp/*/tools/}
        # The mapped interpreter must stay under ~/.hermes/tools: never follow a `..` out of it.
        case "/$rest/" in
            */../*|*/./*)
                say "left $f alone: its interpreter $old is not repointable"
                still_bad=1
                continue ;;
        esac
        new="$tools/$rest"
        if [ -e "$old" ] || [ ! -x "$new" ]; then
            say "left $f alone: its interpreter $old is not repointable"
            still_bad=1
            continue
        fi
        # Write a temp file beside the launcher and move it over, so a running process that is
        # exec'ing the launcher never sees a half-written file. Keep the mode, and keep a one-time
        # backup of the original as the maintainer did by hand.
        backup="$f.before-tmp-fix"
        [ -e "$backup" ] || cp -p "$f" "$backup"
        tmp="$f.tmp.$$"
        # The path holds `.` and `+`; escape every basic-regex metacharacter and the `|` delimiter,
        # so sed matches it literally rather than building a regex from the path.
        pat=$(printf '%s' "$old" | sed 's/[][\.*^$|]/\\&/g')
        rep=$(printf '%s' "$new" | sed 's/[&\\|]/\\&/g')
        sed "s|$pat|$rep|g" "$f" > "$tmp" || { rm -f "$tmp"; still_bad=1; continue; }
        chmod --reference="$f" "$tmp" 2>/dev/null || true
        mv -f "$tmp" "$f" || { rm -f "$tmp"; still_bad=1; continue; }
        say "repointed Hermes's launcher from a temporary Python to ~/.hermes/tools"
    done
    [ "$still_bad" = 0 ]
}
