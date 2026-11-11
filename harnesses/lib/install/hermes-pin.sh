# Which Hermes this repository installs, and the one way it is fetched. Sourced (never run) by
# hermes.sh, which installs it on a machine that has none, and by hermes-bake.sh, which puts it in
# the ISO, so the image and the runtime fallback can never be two different Hermes.
#
# The commit this was last tried against, on Hermes's main line, which is what Hermes's own
# instructions install. The installer is fetched from that commit and told to check out that
# commit, so the script and the tree it sets up always match; `hermes update` moves it on
# afterwards, as it would for anyone. (Release tags carry an older installer that stops on a
# machine with no C++ compiler, which this image is.) Hermes is MIT-licensed.
HERMES_COMMIT=38d3dbce4adbb9074a34214eb330237a129e5904
HERMES_INSTALLER_URL="https://raw.githubusercontent.com/NousResearch/hermes-agent/$HERMES_COMMIT/scripts/install.sh"
# That commit's install.sh, hashed when the pin was moved. Both are bumped together, by hand.
HERMES_INSTALLER_SHA256=9ed50b51fe072df4ece6dc9a72c1ec8d78bc7907eec16a61d8a2a0ef4ec45884

# hermes_run_installer URL SHA256 COMMIT — Hermes's own installer, fetched, checked against SHA256
# and run for COMMIT, without root: it fetches a pinned, hash-checked uv and Python into ~/.hermes,
# clones Hermes into ~/.hermes/hermes-agent and puts `hermes` in ~/.local/bin. A file that does not
# match is never run. `--non-interactive` because there is no terminal (a button, or the image
# build); the browser and computer-use tools are left out: this desktop is the tool surface Hermes
# drives, and Chromium alone is a large download.
hermes_run_installer() {
    command -v git >/dev/null 2>&1 || fail "Hermes needs git"
    command -v bash >/dev/null 2>&1 || fail "Hermes's installer needs bash"
    installer_dir=$(mktemp -d) || fail "no temporary directory"
    trap 'rm -rf "$installer_dir"' EXIT
    curl -fsSL --retry 3 -o "$installer_dir/install.sh" "$1" \
        || fail "could not download Hermes's installer"
    [ "$(sha256sum "$installer_dir/install.sh" | cut -d' ' -f1)" = "$2" ] \
        || fail "Hermes's installer did not match its checksum"
    bash "$installer_dir/install.sh" --commit "$3" --non-interactive \
        --skip-browser --skip-computer-use </dev/null \
        || fail "Hermes's own installer stopped; its last lines are above"
    rm -rf "$installer_dir"; trap - EXIT
}
