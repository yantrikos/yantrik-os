# The two Hermes plugins an install writes into ~/.hermes/plugins, and how each is put in place.
#
# Sourced (never run) by hermes.sh, which writes them at install, and by refresh-plugins.sh, which
# writes them again after an OS update when the build ships different ones. One implementation, so
# a refreshed plugin is byte for byte what a fresh install would have written: before this the copy
# happened once, at install, and every Hermes fix after it (#799, #802) reached fresh installs only
# (VM 541, 14 Oct: the shipped adapter.py had send_clarify, the one Hermes loaded did not).
#
# Callers source common.sh first: these read INSTALL_LIB and MEMORY_WHEELS, and say and fail.
#
#   ~/.hermes/plugins/yantrik     the desktop plugin, a copy of harnesses/hermes as it ships
#   ~/.hermes/plugins/yantrikdb   the YantrikDB memory provider: vendor/ built from the image's
#                                 wheels, its plugin.yaml, and the shim Hermes loads
#
# Nothing else under ~/.hermes is written here: config.yaml, .env, sessions, memories and every
# other plugin are the person's and Hermes's.

# hermes_tree_hash DIR — one sha256 over the files under DIR, by relative path and content.
# Left out: the bytecode Python writes beside a module when Hermes imports it, and each package's
# RECORD, which lists that bytecode with its hashes. Counted, either would make a plugin Hermes has
# run look different from the same plugin freshly written, and every update would replace it.
hermes_tree_hash() {
    ( cd "$1" && find . -type d -name __pycache__ -prune \
        -o -type f ! -name '*.pyc' ! -path '*.dist-info/RECORD' -exec sha256sum {} + ) \
        | LC_ALL=C sort -k 2 | sha256sum | cut -d' ' -f1
}

# hermes_staging NAME — where a plugin NAME is built and where the one it replaces is moved aside:
# ~/.hermes/.yantrik-staging/NAME-new and NAME-old, made if missing. On the same filesystem as
# ~/.hermes/plugins, so each move is a rename; and never inside it, because Hermes 0.21's
# scan_directory (hermes_cli/plugins_discovery.py) skips only dunder directories there: every child
# with a plugin.yaml is a plugin, and of two with the same name the later in sorted order wins. On
# VM 541 (14 Oct) a backup left beside `yantrik` was the one Hermes ran, and a half-finished swap
# that left `.yantrik-old` and `.yantrik-new` there would have run the old one too (review of #808).
hermes_staging() {
    mkdir -p "$HOME/.hermes/.yantrik-staging" && printf '%s\n' "$HOME/.hermes/.yantrik-staging/$1"
}

# hermes_swap_dir NEW DEST — NEW, built in the staging directory, put in DEST's place whole. The
# old one is moved aside first, into the staging directory, and only removed once NEW is in, so a
# failed move puts it back: Hermes finds the earlier plugin or the new one, never half of either,
# never none, and never a second copy under ~/.hermes/plugins.
hermes_swap_dir() {
    old="$(hermes_staging "$(basename "$2")")-old" || return 1
    # DEST's folder, which a fresh install does not have yet: with the staging moved out of
    # ~/.hermes/plugins (above), nothing else made it, and every fresh install failed at
    # "could not put the desktop plugin in ~/.hermes/plugins/yantrik" (VM 542, 15 Oct 2026).
    mkdir -p "$(dirname "$2")" || return 1
    rm -rf "$old" || return 1
    if [ -e "$2" ]; then
        mv "$2" "$old" || return 1
    fi
    if ! mv "$1" "$2"; then
        [ -e "$old" ] && mv "$old" "$2"
        return 1
    fi
    rm -rf "$old"
    rmdir "$HOME/.hermes/.yantrik-staging" 2>/dev/null || true
}

# hermes_desktop_plugin_build SRC NEW — the desktop plugin as an install writes it: SRC (the
# harnesses/hermes directory the build ships) copied whole into a new NEW (hermes_staging yantrik).
hermes_desktop_plugin_build() {
    rm -rf "$2" && mkdir -p "$2" && cp -r "$1/." "$2/"
}

# uv on PATH, else the one a newer Hermes keeps in ~/.hermes/tools: its venv is made by uv and has
# no pip in it (0.21, 13 Oct), so `python -m pip` is only for an older Hermes whose venv has one.
hermes_uv() {
    command -v uv 2>/dev/null && return
    # The newest by modification time: a version in the folder name does not sort as text.
    for candidate in $(ls -td "$HOME"/.hermes/tools/uv-*/uv 2>/dev/null); do
        [ -x "$candidate" ] && { echo "$candidate"; return; }
    done
}

# hermes_vendor_install DIR PYTHON — the provider, the engine and their two dependencies into DIR,
# for Hermes's Python, from the image's wheels only: no index, nothing fetched.
hermes_vendor_install() {
    uv_bin=$(hermes_uv)
    if [ -n "$uv_bin" ]; then
        "$uv_bin" pip install --python "$2" --target "$1" --offline --no-index \
            --find-links "$MEMORY_WHEELS" --no-deps yantrikdb yantrikdb-hermes-plugin uuid-utils click </dev/null
    else
        "$2" -m pip install --target "$1" --no-index \
            --find-links "$MEMORY_WHEELS" --no-deps yantrikdb yantrikdb-hermes-plugin uuid-utils click </dev/null
    fi
}

# hermes_yantrikdb_plugin_build NEW PYTHON — the YantrikDB plugin as an install writes it, into a
# new NEW (hermes_staging yantrikdb), checked to import in Hermes's Python. Calls fail, so a caller that must carry on after
# a failure (refresh-plugins.sh) runs it in a subshell.
#
# Not into Hermes's venv: Hermes 0.21's package manager rebuilds that as a new generation from its
# lock on an update, keeping only what its lock and its plugins' pip_dependencies name, and on VM 541
# (13 Oct) the rebuild that followed an install dropped the provider. So the provider, the engine and
# their two dependencies go into vendor/ beside the plugin's shim (hermes_runtime.VENDOR), and the
# shim puts vendor/ at the END of sys.path: Hermes's own packages always come first, and nothing of
# Hermes's is moved. requests is Hermes's own.
hermes_yantrikdb_plugin_build() {
    new_plugin=$1 hermes_python=$2
    rm -rf "$new_plugin" && mkdir -p "$new_plugin" || fail "could not make $new_plugin"
    hermes_vendor_install "$new_plugin/vendor" "$hermes_python" >/dev/null \
        || fail "the YantrikDB memory provider did not install from $MEMORY_WHEELS"
    # The shim Hermes loads (the package's own `yantrikdb-hermes install` writes the same three
    # lines, without vendor/), and the package's plugin.yaml beside it, which Hermes reads for its
    # hooks. Without its pip_dependencies: vendor/ supplies them, and named there they would have
    # Hermes's own update fetch yantrikdb from PyPI into its venv, ahead of vendor/
    # (hermes_config.py, `manifest`).
    "$hermes_python" "$INSTALL_LIB/hermes_config.py" manifest \
        "$new_plugin/vendor/yantrikdb_hermes_plugin/plugin.yaml" "$new_plugin/plugin.yaml" </dev/null \
        || fail "the YantrikDB memory provider has no plugin.yaml Hermes could be given"
    cat > "$new_plugin/__init__.py" <<'SHIM' || fail "could not write the YantrikDB plugin's shim"
"""Hermes user-plugin shim for the YantrikDB provider, written by Yantrik OS (harnesses/lib/install/hermes.sh).

The provider and its engine are in vendor/ beside this file, not in Hermes's own environment, which
Hermes rebuilds on an update. vendor/ goes at the end of sys.path, so Hermes's own packages come first.
"""
import os
import sys

_VENDOR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "vendor")
if _VENDOR not in sys.path:
    sys.path.append(_VENDOR)

from yantrikdb_hermes_plugin import YantrikDBMemoryProvider  # noqa: E402


def register(ctx):
    ctx.register_memory_provider(YantrikDBMemoryProvider())
SHIM
    "$hermes_python" -c "import sys; sys.path.append(sys.argv[1]); import requests, yantrikdb, yantrikdb_hermes_plugin" \
        "$new_plugin/vendor" </dev/null \
        || fail "the YantrikDB memory provider is installed but does not import in Hermes's Python"
}
