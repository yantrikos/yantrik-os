# What the bake looks for in the tree it is about to ship. Sourced (never run) by hermes-bake.sh;
# each prints the paths it found, one per line, and nothing when all is well.

# What Hermes still owes the first run: the marker of an unfinished source update and the record
# of its failed attempts (hermes_cli/venv_sync.py). prepare_launch finishes that tail on its next
# command, with `npm ci` against the registry, so a tree that carries either is not one that works
# with the network off (review of #1169, L-B). Looked for anywhere under ~/.hermes, node_modules
# left out, because the state directory's place is Hermes's to choose.
hermes_completion_owed() { # <hermes home>
    find "$1" -name node_modules -prune -o \
        \( -name source-completion-pending -o -name source-completion-attempts \) -print
}

# What must not ship and must not come back: the GPL ffmpeg, gsap, and the two download caches.
# Asked after every Hermes command has run, because any of them can fetch it again (L-C).
hermes_unshippable() { # <hermes home> <npm cache>
    for found in "$1"/tools/ffmpeg-* "$1/hermes-agent/node_modules/gsap" "$1/cache/uv" "$2"; do
        [ -e "$found" ] || [ -L "$found" ] && printf '%s\n' "$found"
    done
    return 0
}
