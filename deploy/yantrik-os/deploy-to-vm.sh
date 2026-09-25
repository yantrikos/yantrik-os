#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════
# deploy-to-vm.sh — push the WHOLE OS to a Debian 13 host
# ═══════════════════════════════════════════════════════════════
#
# Why this exists, and why it is not build-debian-iso.sh.
#
# The ISO builder ships two binaries: `yantrik-ui` and `yantrik`. It was
# written when yantrik-os was one binary, and the OS is now a shell plus
# seventeen apps plus nine out-of-process services. So an ISO install gives
# you a desktop whose `open_app` cannot find anything to open, and whose
# service list reports "Binary not found" for every service it names.
#
# FIXED 2026-09-13: build-debian-iso.sh no longer keeps its own list. It now
# installs the artifact build-release.sh produces, which DISCOVERS every
# executable in the release directory, and asserts the full set landed before
# it will build an image. A current ISO ships 31 binaries, not 2.
#
# This script is still the fastest way to push a working tree onto a machine
# that already has Debian and labwc, without rebuilding an image for it.
#
# Usage:
#   ./deploy-to-vm.sh <host>                     # e.g. yantrik@192.168.4.66
#   BACKEND=candle ./deploy-to-vm.sh <host>      # local LLM instead of the API
#
# Assumes on the target:
#   - Debian 13 (trixie) or compatible glibc
#   - labwc, mesa, seatd installed
#   - passwordless sudo for the SSH user
#
# Assumes here:
#   - cargo build --release --workspace --bins has been run
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

TARGET_HOST="${1:?usage: deploy-to-vm.sh user@host}"
# Asked of cargo, not assumed. The same line in build-release.sh shipped a release whose app
# binaries were five hours old: with CARGO_TARGET_DIR unset, cargo writes to ./target while
# this default points at a directory left over from when the repo lived on /mnt/c. Anything
# that copies binaries somewhere has to read the directory cargo actually writes to.
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}"
TARGET_DIR="${TARGET_DIR:-/home/yantrik/target-yantrik}/release"
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SSH_KEY="${SSH_KEY:-$HOME/.ssh/id_deploy}"
BACKEND="${BACKEND:-api}"
REMOTE="/opt/yantrik"

SSH=(ssh -i "$SSH_KEY" -o StrictHostKeyChecking=no "$TARGET_HOST")
RSYNC_RSH="ssh -i $SSH_KEY -o StrictHostKeyChecking=no"

say() { printf '\n\033[36m::\033[0m \033[1m%s\033[0m\n' "$1"; }

[ -x "$TARGET_DIR/yantrik-ui" ] || {
  echo "FAIL: $TARGET_DIR/yantrik-ui missing — run: cargo build --release --workspace --bins"
  exit 1
}

# ── What the OS consists of ──
#
# Discovered rather than listed, because a hardcoded list is exactly how the
# ISO builder fell behind the architecture. Anything the workspace builds
# whose name starts with `yantrik-` or ends in `-service` is part of the OS.
say "Collecting binaries"
mapfile -t BINS < <(
  find "$TARGET_DIR" -maxdepth 1 -type f -executable \
    \( -name 'yantrik*' -o -name '*-service' \) ! -name '*.d' ! -name '*.so' \
    -printf '%f\n' | sort
)
[ "${#BINS[@]}" -gt 0 ] || { echo "FAIL: no binaries found in $TARGET_DIR"; exit 1; }
printf '   %s\n' "${BINS[@]}" | paste -sd' ' - | fold -sw 76 | sed 's/^/   /'
echo "   ${#BINS[@]} binaries"

say "Preparing $TARGET_HOST"
"${SSH[@]}" "sudo mkdir -p $REMOTE/{bin,data,logs,models,config} && sudo chown -R \$(id -u):\$(id -g) $REMOTE"

say "Shipping binaries"
# --inplace so a running binary is not replaced under a live process by an
# unlink-and-rename; the OS is expected to be restarted after this anyway,
# but a half-swapped bin directory is a confusing thing to debug.
for b in "${BINS[@]}"; do printf '%s\n' "$b"; done \
  | rsync -a --info=stats1 --files-from=- -e "$RSYNC_RSH" \
      "$TARGET_DIR/" "$TARGET_HOST:$REMOTE/bin/" 2>&1 | tail -3

# The machine's own record of what is installed. Left alone, it went on naming the last bundle
# that was installed while these binaries ran, so the About screen and `describe shell` reported a
# build that was not running. A developer deploy says what it is: this tree's revision, marked
# -dev. The channel line is kept as it was, because the updater reads its channel from here.
DEV_VERSION="$(git -C "$PROJECT_ROOT" describe --tags --always --dirty 2>/dev/null || echo unknown)-dev"
DEV_GIT="$(git -C "$PROJECT_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
"${SSH[@]}" "B=$REMOTE/BUILD; CH=\$(sed -n 's/^channel=//p' \$B 2>/dev/null | head -1); { echo name=dev-deploy; echo version=$DEV_VERSION; echo git=$DEV_GIT; [ -n \"\$CH\" ] && echo channel=\$CH; echo installed=\$(date -u +%Y-%m-%dT%H:%M:%SZ); } | sudo tee \$B >/dev/null"
echo "   BUILD: $DEV_VERSION"

# rsync does not carry file capabilities; grant perception-service its two on the target the way
# the ISO does, so a dev deploy watches the same way a shipped machine does.
"${SSH[@]}" "sudo setcap cap_sys_admin,cap_net_admin=ep $REMOTE/bin/perception-service 2>/dev/null \
    || echo '   (setcap unavailable or refused: perception-service will watch with PSI only)'"

say "Shipping the agent surface"
# yos is how an agent sees and acts on this desktop; yos-mcp offers the same surface to a
# mind that speaks MCP. Neither is compiled, so neither appears in the binary discovery
# above — without this they exist only on machines where someone copied them by hand.
for f in yos yos-mcp release-check; do
  if [ -f "$SCRIPT_DIR/$f" ]; then
    rsync -a -e "$RSYNC_RSH" "$SCRIPT_DIR/$f" "$TARGET_HOST:$REMOTE/bin/$f"
    "${SSH[@]}" "chmod +x $REMOTE/bin/$f"
    echo "   $f"
  else
    echo "   MISSING: $SCRIPT_DIR/$f"
  fi
done

say "Shipping the speech model"
# Without this, `yos web listen` reports that transcription is unavailable — which is at
# least honest, but the machine is then deaf to anything that is spoken rather than written.
if [ -d /opt/yantrik/models/whisper ]; then
  rsync -a -e "$RSYNC_RSH" /opt/yantrik/models/whisper/ "$TARGET_HOST:$REMOTE/models/whisper/"
  echo "   whisper ($(du -sh /opt/yantrik/models/whisper 2>/dev/null | cut -f1))"
else
  echo "   skipped — /opt/yantrik/models/whisper not present here"
fi

say "Shipping the embedder"
if [ -d /opt/yantrik/models/embedder ]; then
  rsync -a -e "$RSYNC_RSH" /opt/yantrik/models/embedder/ "$TARGET_HOST:$REMOTE/models/embedder/"
  echo "   MiniLM-L6-v2 (384-dim)"
else
  echo "   skipped — /opt/yantrik/models/embedder not present here"
fi

say "Desktop chrome"
"${SSH[@]}" "mkdir -p $REMOTE/share/labwc $REMOTE/share/fonts"
rsync -a -e "$RSYNC_RSH" "$PROJECT_ROOT/config/labwc/" "$TARGET_HOST:$REMOTE/share/labwc/"
rsync -a -e "$RSYNC_RSH" "$PROJECT_ROOT/config/labwc-mind/" "$TARGET_HOST:$REMOTE/share/labwc-mind/"
rsync -a -e "$RSYNC_RSH" "$PROJECT_ROOT/crates/yantrik-design-tokens/slint/fonts/" \
  "$TARGET_HOST:$REMOTE/share/fonts/"
echo "   labwc theme + Barlow/JetBrains Mono"

say "Application entries (.desktop): how the shell finds this OS's apps"
# The shell has no table of our apps any more: each one declares its control surface, what it is
# for and its other names in its own .desktop file (X-Yantrik-Surface / -Purpose / -Aliases), the
# way an app somebody else wrote does. This script shipped the binaries and not those files, so a
# machine deployed with it kept whatever entries an older release left — VM 520 had 14 of 17, none
# with the keys: Arcade could not be opened at all, and no app of ours was listed while closed,
# answered to an alias, or could have a notification button carried out.
#
# The same set a release installs (shipped-desktop-files.sh: everything in apps/desktop-files but a
# shelved app's), into $REMOTE/share/applications, which the session puts on XDG_DATA_DIRS and the
# shell searches regardless. Through /tmp and `sudo install`, so a file some earlier deploy left
# owned by root is replaced rather than refused.
#
# A copy an older deploy put in /usr/share/applications comes first in the search order and would
# shadow the new one with its old keys, so any such copy of one of these entries is replaced too.
# Nothing else there is touched.
mapfile -t DESKTOP < <("$SCRIPT_DIR/shipped-desktop-files.sh")
[ "${#DESKTOP[@]}" -gt 0 ] || { echo "FAIL: no .desktop files to ship"; exit 1; }
STAGE="/tmp/yantrik-desktop-files.$$"
"${SSH[@]}" "rm -rf $STAGE && mkdir -p $STAGE"
rsync -a -e "$RSYNC_RSH" "${DESKTOP[@]}" "$TARGET_HOST:$STAGE/"
"${SSH[@]}" "set -e
  sudo install -d -m 755 -o \$(id -u) -g \$(id -g) $REMOTE/share/applications
  sudo install -m 644 -o \$(id -u) -g \$(id -g) $STAGE/*.desktop $REMOTE/share/applications/
  for f in $STAGE/*.desktop; do
    old=/usr/share/applications/\$(basename \"\$f\")
    if [ -e \"\$old\" ]; then sudo install -m 644 \"\$f\" \"\$old\"; echo \"   replaced the older copy in \$old\"; fi
  done
  rm -rf $STAGE"
printf '%s\n' "${DESKTOP[@]##*/}" | paste -sd' ' - | fold -sw 76 | sed 's/^/   /'
echo "   ${#DESKTOP[@]} entries -> $REMOTE/share/applications (the shell picks them up within seconds)"

say "Config (backend: $BACKEND)"
CONFIG_SRC="$PROJECT_ROOT/config/yantrik-ollama.yaml"
[ "$BACKEND" = "candle" ] && CONFIG_SRC="$PROJECT_ROOT/config/yantrik-os.yaml"
rsync -a -e "$RSYNC_RSH" "$CONFIG_SRC" "$TARGET_HOST:$REMOTE/config.yaml"
echo "   $(basename "$CONFIG_SRC") -> $REMOTE/config.yaml"

# ── The session ──
#
# labwc on the headless wlroots backend WHEN THE MACHINE HAS NO SCREEN. This
# deployment was written for test VMs with no GPU and no monitor, where the
# point is the control surface, not the pixels: the apps are real Slint
# windows either way, rendering into a buffer nobody looks at, which is enough
# for `app.describe` to tell the truth about them.
#
# It used to force headless unconditionally. Run against a VM that DOES have a
# display — VM 520, with a virtio screen and a person at the Proxmox console —
# that wrote a session script that put the whole desktop on an output no screen
# shows: the console sat on tty1's login banner, `wlr-randr` said HEADLESS-1,
# nobody held /dev/dri/card0, and every approval card expired unseen, because
# there was nowhere to see it. The running session survived until the next
# reboot, so the damage surfaced hours after the deploy that caused it (#96's
# afternoon, 2026-09-22).
#
# So the script the target gets now decides at session start, from the one
# fact that settles it: whether any DRM connector reports `connected`. A screen
# means the real backend; none means headless, exactly as before.
say "Session launcher"
"${SSH[@]}" "cat > $REMOTE/bin/yantrik-session" <<'SESSION'
#!/bin/sh
# Start labwc, then the shell inside it. Headless only if there is no screen.
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
mkdir -p "$XDG_RUNTIME_DIR"
if [ "${YANTRIK_HEADLESS:-}" = "1" ] || ! grep -qs '^connected' /sys/class/drm/card*-*/status; then
  # No connected display (or headless asked for): render into a buffer.
  export WLR_BACKENDS=headless
  export WLR_LIBINPUT_NO_DEVICES=1
fi
export LIBGL_ALWAYS_SOFTWARE=1
export PATH="/opt/yantrik/bin:$PATH"


# The window decorations and the typeface they are drawn in.
#
# labwc reads its config from the user's home, so this needs no root and re-applies on every
# start — a machine that was deployed before the theme existed picks it up by rebooting. Both
# sources live in the release payload; see deploy/yantrik-os/build-release.sh.
CHROME=/opt/yantrik/share
if [ -d "$CHROME/labwc" ]; then
  mkdir -p "$HOME/.config/labwc" "$HOME/.local/share/themes/Yantrik/labwc"
  cp -f "$CHROME/labwc/rc.xml" "$HOME/.config/labwc/rc.xml"
  cp -f "$CHROME/labwc/themerc" "$HOME/.local/share/themes/Yantrik/labwc/themerc"
fi
if [ -d "$CHROME/fonts" ]; then
  mkdir -p "$HOME/.local/share/fonts"
  cp -f "$CHROME/fonts/"*.ttf "$HOME/.local/share/fonts/" 2>/dev/null
  fc-cache -f "$HOME/.local/share/fonts" >/dev/null 2>&1
fi

mkdir -p /opt/yantrik/logs
exec labwc -s '/opt/yantrik/bin/yantrik-shell /opt/yantrik/config.yaml' \
  >> /opt/yantrik/logs/session.log 2>&1
SESSION
"${SSH[@]}" "chmod +x $REMOTE/bin/yantrik-session"
# What the session runs in the shell's place: it starts the shell again when it dies (#247).
"${SSH[@]}" "cat > $REMOTE/bin/yantrik-shell && chmod +x $REMOTE/bin/yantrik-shell" < "$SCRIPT_DIR/yantrik-shell"

say "Done: binaries, yos/yos-mcp/release-check, models, chrome, .desktop entries, session"
"${SSH[@]}" "ls $REMOTE/bin | tr '\n' ' '; echo; echo; echo 'start:  setsid $REMOTE/bin/yantrik-session &'"
