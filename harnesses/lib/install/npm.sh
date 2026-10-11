#!/bin/sh
# Install a harness that npm publishes: npm.sh PACKAGE COMMAND NODE_MIN [npm options...]
#
# Into ~/.local, so COMMAND lands in ~/.local/bin beside the Node it runs on, and no step needs
# root. Node itself is fetched first when the machine has none new enough, which on a fresh
# image is always: the image ships none.
set -eu
. "$(dirname "$0")/common.sh"

[ $# -ge 3 ] || fail "usage: npm.sh PACKAGE COMMAND NODE_MIN [npm options]"
package=$1 command=$2 node_min=$3
shift 3

ensure_node "$node_min"
say "fetching $package"
npm install --global --prefix "$HOME/.local" --no-fund --no-audit --no-update-notifier "$@" "$package" \
    || fail "npm could not install $package"
[ -x "$USER_BIN/$command" ] || fail "$package installed, but there is no $command in ~/.local/bin"
version=$("$USER_BIN/$command" --version 2>/dev/null | head -n 1) || true
say "$command ${version:-is} installed in ~/.local/bin"
