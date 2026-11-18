#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# publish.sh — hand one build to the public server, which decides what happens to it
# ═══════════════════════════════════════════════════════════════════════════════════════
#
#   publish.sh iso       <channel> <file.iso>
#   publish.sh release   <channel> <bundle.tar.zst> <version> <git> <binaries>
#   publish.sh changelog <channel> <yantrik-os-<version>.changelog.md>
#   publish.sh manifest  <channel> <release.signed>   (made by sign-manifest.sh, #537)
#
# The other end is deploy/yantrik-os/server/yantrik-publish, installed as the forced command
# of the key this uses. This script streams the file and says what it is; the server hashes
# what arrived, refuses a mismatch, moves it into place, repoints `latest`, rewrites the
# manifest and prunes the channel to the newest three. None of that is done from here, on
# purpose: a client that prunes is a client that can delete, and the CI key cannot.
#
#   YANTRIK_PUBLISH_HOST   default 15.204.233.63 (iso. and releases.yantrikos.com)
#   YANTRIK_PUBLISH_USER   default ubuntu
#   YANTRIK_PUBLISH_KEY    path to the private key; default ~/.ssh/yantrik_publish
#
# The server's host key is pinned below rather than learned on first use. A pipeline that
# accepts whatever key answers will upload the release to whoever answers.
set -euo pipefail

KIND="${1:?iso, release, changelog or manifest}"; CHANNEL="${2:?channel}"; FILE="${3:?file}"
shift 3
HOST="${YANTRIK_PUBLISH_HOST:-15.204.233.63}"
USER_="${YANTRIK_PUBLISH_USER:-ubuntu}"
KEY="${YANTRIK_PUBLISH_KEY:-$HOME/.ssh/yantrik_publish}"
HOSTKEY="15.204.233.63 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFC+hXclbIm0YNV2Jrn7BF5sO7XbH67cYPq5aP9xuyrQ"

[ -f "$FILE" ] || { echo "publish.sh: no such file: $FILE" >&2; exit 1; }
[ -f "$KEY" ] || { echo "publish.sh: no key at $KEY (set YANTRIK_PUBLISH_KEY)" >&2; exit 1; }

KNOWN="$(mktemp)"; trap 'rm -f "$KNOWN"' EXIT
printf '%s\n' "$HOSTKEY" > "$KNOWN"

SHA="$(sha256sum "$FILE" | cut -d' ' -f1)"
NAME="$(basename "$FILE")"
# The signed manifest is stored under one fixed name, whatever the local file is called.
[ "$KIND" != manifest ] || NAME=release.signed
echo "publishing $KIND/$CHANNEL/$NAME"
echo "  sha256 $SHA  ($(stat -c %s "$FILE") bytes)"

# shellcheck disable=SC2029  # the arguments are meant to be expanded here, not there
ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes \
    -o UserKnownHostsFile="$KNOWN" -o StrictHostKeyChecking=yes \
    -o ServerAliveInterval=30 -o ServerAliveCountMax=10 \
    "$USER_@$HOST" "$KIND" "$CHANNEL" "$NAME" "$SHA" "$@" < "$FILE"

# What was published is what gets downloaded: fetch the checksum the server wrote, over the
# public name, and compare it with the file in hand.
case "$KIND" in
  iso)       URL="https://iso.yantrikos.com/$CHANNEL/$NAME.sha256" ;;
  changelog) URL="https://iso.yantrikos.com/$CHANNEL/$NAME.sha256" ;;
  release)   URL="https://releases.yantrikos.com/$CHANNEL/$NAME.sha256" ;;
  manifest)  URL="https://releases.yantrikos.com/$CHANNEL/release.signed" ;;
esac
if [ "$KIND" = manifest ]; then
  # No .sha256 beside it: the file itself is what machines verify, so fetch that and hash it.
  SERVED="$(curl -fsS --max-time 30 "$URL" | sha256sum | cut -d' ' -f1)"
else
  SERVED="$(curl -fsS --max-time 30 "$URL" | cut -d' ' -f1)"
fi
if [ "$SERVED" = "$SHA" ]; then
  echo "  verified over https: $URL"
else
  echo "publish.sh: $URL serves '$SERVED', expected $SHA" >&2
  exit 1
fi
