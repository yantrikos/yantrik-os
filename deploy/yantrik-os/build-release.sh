#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# build-release.sh — package a built workspace into the tarball everything else installs
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# One artifact, three consumers: the ISO unpacks it, cloud-init fetches it, and
# deploy-to-vm.sh could use it instead of rsyncing a directory. Before this existed each
# of those knew its own list of binaries, and the ISO's list was five months out of date —
# it shipped a two-binary OS with none of the agent surface, and booted a desktop that
# looked right and could do nothing.
#
# So the list is DISCOVERED, never written down. Anything executable in the release
# directory is part of the OS; a new service is packaged because it exists, not because
# someone remembered to add it here.
#
#   ./build-release.sh [--out DIR] [--models] [--no-build]
#
# --models includes the embedder and whisper weights (~235 MB). Without it the tarball is
# just code, which is what a machine that already has the models wants.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# Where cargo ACTUALLY puts things, asked of cargo rather than assumed.
#
# This used to read "${CARGO_TARGET_DIR:-$HOME/target-yantrik}/release". On a machine with
# CARGO_TARGET_DIR unset -- which is the normal case, and was the case on the build box --
# cargo writes to $PROJECT_ROOT/target/release while this script packaged $HOME/target-yantrik.
# It built one directory and shipped another, reported "29 binaries" and a green publish, and
# put a release on the server whose app binaries were five hours old. Every UI change in it was
# missing, and nothing anywhere said so.
#
# `cargo metadata` is the authoritative answer: it accounts for the environment variable, for
# build.target-dir in any .cargo/config.toml, and for the default. An explicit TARGET_DIR still
# wins, for the case where someone is packaging binaries built elsewhere on purpose.
resolve_target_dir() {
  if [ -n "${TARGET_DIR:-}" ]; then printf '%s' "$TARGET_DIR"; return; fi
  local d
  d="$(cd "$PROJECT_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
       | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
  [ -n "$d" ] || d="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"
  printf '%s/release' "$d"
}
TARGET_DIR="$(resolve_target_dir)"
OUT_DIR="$PROJECT_ROOT/dist"
WITH_MODELS=0
DO_BUILD=1

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT_DIR="$2"; shift 2 ;;
    --models) WITH_MODELS=1; shift ;;
    --no-build) DO_BUILD=0; shift ;;
    --publish) PUBLISH_CHANNEL="$2"; shift 2 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31mFAIL: %s\033[0m\n' "$*" >&2; exit 1; }

# The version, computed once here and handed to everything downstream.
#
# YANTRIK_VERSION is honoured so a caller that has already computed it — the ISO workflow does,
# for the image name — gets the identical string in the tarball, the BUILD marker and the
# binaries. A step that recomputed it could land on a different answer than the step before it:
# a tag pushed between the two is enough.
VERSION="${YANTRIK_VERSION:-$(git -C "$PROJECT_ROOT" describe --tags --always --dirty 2>/dev/null)}"
[ -n "$VERSION" ] || fail "cannot determine a version — refusing to build an unidentifiable release"
STAMP="$(date -u +%Y%m%d)"
GITREV="$(git -C "$PROJECT_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
NAME="yantrik-os-${VERSION}-${STAMP}-${GITREV}-linux-amd64"

if [ "$DO_BUILD" = 1 ]; then
  # Package the directory the build writes to. Not "a directory that usually is it".
  #
  # A first version of this check compared file timestamps against a marker made before the
  # build, and failed when nothing was newer. That is wrong: an up-to-date incremental build
  # legitimately rewrites nothing, and the check turned a correct no-op build into an error.
  # The invariant worth enforcing is not "files changed", it is "the directory being packaged
  # is the directory cargo writes to".
  CARGO_DIR="$(cd "$PROJECT_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
               | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')/release"
  if [ -n "$CARGO_DIR" ] && [ "$CARGO_DIR" != "/release" ] && [ "$CARGO_DIR" != "$TARGET_DIR" ]; then
    fail "this build writes to $CARGO_DIR but the packaging step reads $TARGET_DIR.
   Those must be the same directory or the release ships binaries the build never touched —
   which is exactly how a green publish came to contain app binaries five hours old.
   Use --no-build if you mean to package binaries that were built elsewhere."
  fi

  say "Building the workspace"
  # One rustc in this workspace peaks near 14 GB of RSS (Slint macro expansion), so this
  # is the step that decides what machine can build a release at all.
  #
  # YANTRIK_VERSION goes in so crates/yantrik-version bakes THIS string rather than running
  # `git describe` again for itself. The binaries read the installed BUILD marker first, so the
  # baked value only shows up where there is no marker to read — but a fallback that disagrees
  # with the release it was cut from is a fourth answer waiting to be found.
  ( cd "$PROJECT_ROOT" && YANTRIK_VERSION="$VERSION" RUSTFLAGS="-A warnings" cargo build --release --workspace ) \
    || fail "cargo build failed"
fi

[ -d "$TARGET_DIR" ] || fail "no release directory at $TARGET_DIR (set CARGO_TARGET_DIR)"

# The apps that are in the tree and not in this build.
#
# Discovery is the right default and it has one blind spot: a shelved app is still a workspace
# member, on purpose — it keeps compiling and cannot rot in silence — so `cargo build --workspace`
# produces its binary and `find` packages it because it exists. Shipping it would put the app back
# on every machine while the launcher refuses to open it.
#
# The list that decides what is shelved is SHELVED in crates/yantrik-ui/src/wire/dock.rs, where
# each entry carries the reason and what would bring the app back; design/shelved-2026-09-20.md
# is the account. These two names are the same names, and a test in dock.rs reads this file and
# fails if the two stop agreeing. Un-shelving an app is deleting it from both.
#
# This used to be a literal here, copied by hand out of dock.rs. It was also copied into
# deploy.sh, install.sh, scripts/package-all.sh and scripts/publish-components.sh, where it was
# never updated — so the release tarball dropped the shelved apps and every other path put them
# straight back. shelved-bins.sh reads dock.rs, so there is nothing left to copy.
SHELVED_BINS="$("$SCRIPT_DIR/shelved-bins.sh" | paste -sd' ' -)" \
  || fail "cannot determine which apps are shelved — refusing to ship a list nobody checked"

say "Discovering what the OS is made of"
echo "   from $TARGET_DIR"
# `! -name ".*"`: cargo's own `.cargo-lock` sits in the release directory with mode 755, so
# `-type f -executable` matched it and it was staged into bin/ and counted as one of the
# binaries this OS is made of. Every tarball built so far carries it.
mapfile -t BINS < <(
  find "$TARGET_DIR" -maxdepth 1 -type f -executable \
    ! -name ".*" ! -name "*.so" ! -name "*.d" ! -name "*.rlib" ! -name "build-script*" ! -name "test-*" ! -name "*-test" ! -name "bench-*" \
    -printf '%f\n' | sort | grep -vxF "$(printf '%s\n' $SHELVED_BINS)"
)
[ "${#BINS[@]}" -gt 0 ] || fail "no binaries found in $TARGET_DIR"
for b in $SHELVED_BINS; do
  if [ -f "$TARGET_DIR/$b" ]; then echo "   (shelved, not shipped: $b)"; fi
done
printf '   %s\n' "${BINS[@]}" | paste -sd' ' - | fold -sw 76 | sed 's/^/   /'
echo "   ${#BINS[@]} binaries"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ROOT="$STAGE/$NAME"
mkdir -p "$ROOT/bin" "$ROOT/config" "$ROOT/models"

say "Staging"
for b in "${BINS[@]}"; do cp "$TARGET_DIR/$b" "$ROOT/bin/$b"; done

# The agent surface is not compiled, so binary discovery cannot find it. Without these the
# machine boots a desktop that no agent can see or drive — the exact failure the old ISO had.
for f in yos yos-mcp release-check; do
  [ -f "$SCRIPT_DIR/$f" ] || fail "missing $SCRIPT_DIR/$f — the agent surface is not optional"
  cp "$SCRIPT_DIR/$f" "$ROOT/bin/$f"
  chmod +x "$ROOT/bin/$f"
done
echo "   + yos, yos-mcp, release-check"

# The page reader `yos web` evaluates in the browser. It sits beside yos because yos looks for
# it there. It was referenced from the day `yos web` was written and never committed, so every
# published build answered web_read and web_find with "scan.js is missing next to yos".
[ -f "$SCRIPT_DIR/scan.js" ] || fail "missing $SCRIPT_DIR/scan.js — yos web cannot read a page without it"
cp "$SCRIPT_DIR/scan.js" "$ROOT/bin/scan.js"
echo "   + scan.js"

# The updater ships in the image so a machine can update itself. It is a script, not a
# compiled binary, so binary discovery does not find it either — and a machine that cannot
# pull the next build is a machine that gets hand-patched over ssh forever.
if [ -f "$SCRIPT_DIR/yantrik-update" ]; then
  cp "$SCRIPT_DIR/yantrik-update" "$ROOT/bin/yantrik-update"
  chmod +x "$ROOT/bin/yantrik-update"
  echo "   + yantrik-update"
else
  echo "   (no yantrik-update script found — image will not self-update)"
fi

# The session script. It decides what environment every program on this desktop inherits,
# and it used to live only inside cloud-init's write_files -- written once at provision
# time and unfixable thereafter. Shipping it here is what makes the session updatable.
if [ -f "$SCRIPT_DIR/yantrik-session" ]; then
  cp "$SCRIPT_DIR/yantrik-session" "$ROOT/bin/yantrik-session"
  chmod +x "$ROOT/bin/yantrik-session"
  echo "   + yantrik-session"
  # What the session runs in the shell's place, so a crash brings the shell back (#247).
  cp "$SCRIPT_DIR/yantrik-shell" "$ROOT/bin/yantrik-shell"
  chmod +x "$ROOT/bin/yantrik-shell"
  echo "   + yantrik-shell"
else
  fail "missing $SCRIPT_DIR/yantrik-session -- a release without a session does not boot"
fi

# The config a release carries is the public default: no name, no private address, the model
# endpoint on loopback. It used to be config/yantrik-ollama.yaml, the DEV config, which names an
# address on the author's LAN as the model endpoint and the author in the system prompt — right
# for the machine it was written for, and on a stranger's install a mind pointed at an address
# it cannot reach, greeting them by someone else's name. The reason given for leaving it was
# that the nightly channel feeds the author's own VMs; but `yantrik-update` installs bin/ and
# share/ and never touches an installed machine's config.yaml, so nothing of theirs depended on
# it. A first install that does want another config says so: YANTRIK_RELEASE_CONFIG=<path>.
RELEASE_CONFIG="${YANTRIK_RELEASE_CONFIG:-$SCRIPT_DIR/config-default.yaml}"
cp "$RELEASE_CONFIG" "$ROOT/config.yaml"   || fail "missing $RELEASE_CONFIG -- a release without a config does not start"
echo "   + config.yaml  ($(basename "$RELEASE_CONFIG"))"

# Checked all the same, because the override exists and a release is a thing that leaves.
if [ -f "$ROOT/config.yaml" ]; then
  LEAKS="$(grep -nE '192\.168\.|10\.[0-9]+\.[0-9]+\.[0-9]+|172\.(1[6-9]|2[0-9]|3[01])\.|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}|(api[_-]?key|token|secret)[[:space:]]*:' "$ROOT/config.yaml" || true)"
  if [ -n "$LEAKS" ]; then
    printf '   \033[33m!\033[0m config.yaml in this tarball is not fit to publish:\n'
    printf '%s\n' "$LEAKS" | sed 's/^/       /'
    printf '       (set by YANTRIK_RELEASE_CONFIG; do not publish this tarball)\n'
  fi
fi

# The desktop's own chrome. Without these the compositor runs on stock defaults and draws a
# light-grey title bar, in a font this OS does not use, around every one of its dark apps — the
# most-seen pixels on the machine, and the last ones anybody thought to own. The session script
# installs them on start; they ship here because a release that cannot dress its own windows is
# not a release of this OS.
mkdir -p "$ROOT/share/labwc" "$ROOT/share/fonts"
cp "$PROJECT_ROOT/config/labwc/rc.xml" "$ROOT/share/labwc/rc.xml"
cp "$PROJECT_ROOT/config/labwc/themerc" "$ROOT/share/labwc/themerc"
# The titlebar buttons, which labwc loads from the theme directory in place of its built-in
# six-by-six bitmaps. Rendered by scripts/render-window-buttons.py. Not optional decoration: the
# built-ins lit 64 pixels between the three of them on this palette, which is how a machine
# comes to have no visible way to close a window.
cp "$PROJECT_ROOT/config/labwc/"*.png "$ROOT/share/labwc/" 2>/dev/null \
  || fail "no titlebar button icons in config/labwc — run scripts/render-window-buttons.py"
# What starts with the desktop: the polkit agent, and the note saying why no notification
# daemon is started beside it — the notifications service holds that bus name.
cp "$PROJECT_ROOT/config/labwc/autostart" "$ROOT/share/labwc/autostart"
# Mind View's own compositor (#239): the nested labwc a mind's apps are drawn in. yantrik-ui runs
# it with -C on this directory; without it, a mind's apps open on the person's desktop as before.
mkdir -p "$ROOT/share/labwc-mind"
cp "$PROJECT_ROOT/config/labwc-mind/rc.xml" "$ROOT/share/labwc-mind/rc.xml"
# Barlow is embedded in each app binary, which the compositor cannot read a font out of, so the
# same files also ship loose for fontconfig.
cp "$PROJECT_ROOT/crates/yantrik-design-tokens/slint/fonts/"*.ttf "$ROOT/share/fonts/"
# The applications this OS ships, as .desktop entries.
#
# They existed in the repository and were installed nowhere, so the launcher -- the "all
# applications" view -- listed fourteen shell screens, Chromium, Vim and Print Settings, and
# not one of the sixteen apps this OS is made of. Photographed saying "Search 17 applications"
# with no Notes, no Mail, no Terminal in it.
#
# They go under $ROOT/share so the session can add /opt/yantrik/share to XDG_DATA_DIRS and the
# ordinary freedesktop scan finds them. No root, no writing into /usr, and the same mechanism
# every other application on the machine uses.
#
# A shelved app's entry stays in the repository and is not installed. An installed entry is all
# the launcher needs to list an app -- the catalogue rescans the applications directories every
# time it opens -- so shipping one for an app the shell refuses to open would put the tile back
# on the screen and leave the click doing nothing.
#
# Which entries those are is shipped-desktop-files.sh's answer, the same one deploy-to-vm.sh
# ships, so a release and a developer's deploy cannot install different sets.
mkdir -p "$ROOT/share/applications"
SHIPPED_DESKTOP="$("$SCRIPT_DIR/shipped-desktop-files.sh")" \
  || fail "could not tell which .desktop files ship (shipped-desktop-files.sh)"
while IFS= read -r f; do
  cp "$f" "$ROOT/share/applications/"
done <<< "$SHIPPED_DESKTOP"
[ -n "$(ls -A "$ROOT/share/applications" 2>/dev/null)" ] \
  || fail "no .desktop files to ship — the launcher would not list this OS's own apps"
echo "   + $(ls "$ROOT/share/applications" | wc -l) application entries"

# The icon those entries name.
#
# Every one of them says `Icon=yantrik`, and until this the machine had no file by that name
# anywhere on the icon path — so sixteen correct .desktop entries drew sixteen blank tiles.
# An Icon= key that resolves to nothing does not fall back to a generic icon; it falls back
# to a hole.
#
# It goes under $ROOT/share/icons, beside share/applications, because the session puts
# /opt/yantrik/share on XDG_DATA_DIRS and the freedesktop icon lookup walks
# $XDG_DATA_DIRS/icons/hicolor/<size>/apps/<name>. Same directory, same mechanism, no root
# and nothing written into /usr. The SVG is what a scalable theme wants; the three PNGs are
# for the toolkits that will not read one.
ICONS="$ROOT/share/icons/hicolor"
mkdir -p "$ICONS/scalable/apps"
cp "$PROJECT_ROOT/brand/yantrik-mark.svg" "$ICONS/scalable/apps/yantrik.svg" \
  || fail "brand/yantrik-mark.svg missing — the apps would ship with an Icon= that resolves to nothing"
for px in 48 128 256; do
  mkdir -p "$ICONS/${px}x${px}/apps"
  cp "$PROJECT_ROOT/brand/yantrik-mark-${px}.png" "$ICONS/${px}x${px}/apps/yantrik.png" \
    || fail "brand/yantrik-mark-${px}.png missing — run: python3 brand/render.py"
done
echo "   + app icon (svg + 48/128/256) as hicolor 'yantrik'"

# The Blender addon and its bootstrap.
#
# Blender itself is not built or shipped here — it is a program the machine has, or does
# not. What the release carries is the half that makes it an app of this desktop:
# bootstrap.py, which the launcher starts Blender with; the yantrik_blender addon the
# bootstrap imports, which binds app-blender.sock and answers app.describe / app.act like
# every app this OS builds; and the surface SDK (sdk/python/yantrik_surface) the addon is
# built on, vendored beside it. Vendored rather than installed into a site-packages, because
# a blender.org build runs its own Python that sees no system packages, and so that the addon
# always runs with the SDK it was released with. The dock route (Launch::Blender in
# wire/dock.rs) and the .desktop entry both name this exact path; if this block moves, both
# move with it.
mkdir -p "$ROOT/share/blender"
cp "$PROJECT_ROOT/apps/blender/bootstrap.py" "$ROOT/share/blender/bootstrap.py" \
  || fail "apps/blender/bootstrap.py missing — the launcher would open Blender with no way to talk to it"
cp -r "$PROJECT_ROOT/apps/blender/addon/yantrik_blender" "$ROOT/share/blender/" \
  || fail "apps/blender/addon/yantrik_blender missing — the bootstrap would import nothing"
cp -r "$PROJECT_ROOT/sdk/python/yantrik_surface" "$ROOT/share/blender/" \
  || fail "sdk/python/yantrik_surface missing — the Blender addon would have no surface to serve"
find "$ROOT/share/blender" -name __pycache__ -type d -prune -exec rm -rf {} +
echo "   + blender control-surface addon (bootstrap + yantrik_blender + yantrik_surface SDK)"

# The agent catalog's shipped roles (design/desk-and-mind-2026-09-23.md, section 5): Researcher,
# Planner, Coder, Reviewer, Red team, Writer, Chair, Scribe. The shell has the same files compiled
# in (crates/yantrik-ui/src/agents/catalog.rs); these copies are the image's layer over them, and
# what a person reads and copies into ~/.config/yantrik/agents/ to make a role their own.
mkdir -p "$ROOT/share/agents"
cp "$PROJECT_ROOT/config/agents/"*.toml "$ROOT/share/agents/" \
  || fail "config/agents/*.toml missing — the image would ship no agent catalog of its own"
echo "   + $(ls "$ROOT/share/agents" | wc -l) agent catalog roles"

echo "   + labwc theme and $(ls "$ROOT/share/fonts" | wc -l) fonts"

# ── Nothing in this bundle may have CRLF line endings ──
#
# The scripts above — yos, yos-mcp, yantrik-update, yantrik-session, the labwc autostart — are
# copied verbatim out of a working tree that is edited on Windows. A `#!/usr/bin/env python3`
# line ending in CR names an interpreter called `python3\r`, which does not exist, so the file
# is installed, is executable, and fails on its first line with "no such file or directory".
# This has already shipped once: .gitattributes carries the account of it.
#
# git's eol=lf does not save us here. The blobs in this repository were committed with CRLF
# before those attributes existed, and checkout converts LF to the platform ending — it does
# not strip CRs that are already in the blob. So the check has to be on the bytes being packed.
#
# Stripped rather than failed: refusing to build would block every release made from a Windows
# checkout, which is most of them. Named out loud, so it is a thing someone can go and fix at
# the source rather than a silent repair that runs forever.
say "Checking line endings"
CRLF_FIXED=""
for f in "$ROOT/bin/"* "$ROOT/share/labwc/autostart"; do
  [ -f "$f" ] || continue
  # Text only: the compiled binaries are full of 0x0d and must not be touched.
  head -c 2 "$f" | grep -q '^#!' || continue
  if grep -qU $'\r' "$f" 2>/dev/null; then
    sed -i 's/\r$//' "$f"
    CRLF_FIXED="$CRLF_FIXED $(basename "$f")"
  fi
done
if [ -n "$CRLF_FIXED" ]; then
  printf '   \033[33m!\033[0m CRLF stripped from:%s\n' "$CRLF_FIXED"
  printf '       These would have been installed unrunnable. Fix at the source:\n'
  printf '       git add --renormalize . && git commit\n'
else
  echo "   all shipped scripts are LF"
fi

if [ "$WITH_MODELS" = 1 ]; then
  say "Including models"
  for m in embedder whisper; do
    if [ -d "/opt/yantrik/models/$m" ]; then
      cp -r "/opt/yantrik/models/$m" "$ROOT/models/$m"
      echo "   $m ($(du -sh "$ROOT/models/$m" | cut -f1))"
    else
      echo "   $m not present locally — skipped"
    fi
  done
fi

# A manifest, so a running machine can say what it is. "Which build is this?" was
# unanswerable on the VM all day; a version string in a file costs nothing and settles it.
#
# ── No update.conf in the bundle, and no `channel=` in this marker ──
#
# Considered and rejected. The bundle is built once and can be published to more than one
# channel — `--publish` is a separate step further down this same script, and it can be run
# twice against the same tarball. A channel name baked in here would be right for whichever
# channel it was published to first and a lie for every other one, and it would be a lie that
# `yantrik-update` believes: the updater falls back to BUILD's `channel=` when update.conf is
# silent, so a wrong value here is worse than no value.
#
# The channel is known by the thing that DOWNLOADS the bundle, because it downloaded it from a
# channel URL. cloud-init derives it from YANTRIK_RELEASE_URL and writes update.conf;
# yantrik-install.sh derives it from the image it installed; the ISO build writes it outright.
# `yantrik-update apply` adds `channel=` to BUILD when it installs, because at that moment it
# does know. Nobody guesses.
cat > "$ROOT/BUILD" <<EOF
name=$NAME
version=$VERSION
git=$GITREV
built=$(date -u +%Y-%m-%dT%H:%M:%SZ)
binaries=${#BINS[@]}
models=$([ "$WITH_MODELS" = 1 ] && echo included || echo excluded)
EOF

# ── What changed ──
#
# Every build carries its own changelog: the non-merge commit subjects since the build that was
# published before it. It lands at share/CHANGELOG.md in the payload, so an installed machine
# has it at /opt/yantrik/share/CHANGELOG.md, where the About screen reads it and a person can
# `cat` it. The Discord announcement and the workflow summary print the same list.
#
# "Since the previous build" is a fact the ISO workflow knows — its `decide` job reads the
# channel's latest.json and hands the git hash in as YANTRIK_PREVIOUS. A local build has no such
# fact; it falls back to the last tag, and failing that to the last forty changes, and says which
# it did in the file rather than presenting a guess as a range.
PREVIOUS="${YANTRIK_PREVIOUS:-}"
if [ -z "$PREVIOUS" ]; then
  PREVIOUS="$(git -C "$PROJECT_ROOT" describe --tags --abbrev=0 HEAD~1 2>/dev/null || true)"
fi
if [ -n "$PREVIOUS" ] && git -C "$PROJECT_ROOT" rev-parse -q --verify "$PREVIOUS^{commit}" >/dev/null 2>&1; then
  CHANGE_RANGE="$PREVIOUS..HEAD"
  CHANGE_SINCE="since $PREVIOUS"
else
  CHANGE_RANGE="-n 40"
  CHANGE_SINCE="the last 40 changes (no previous build was known when this was built)"
fi
mkdir -p "$ROOT/share"
{
  echo "# What changed in $VERSION"
  echo
  echo "_${CHANGE_SINCE}; built $(date -u +%Y-%m-%d), git $GITREV._"
  echo
  # git stops at 200 itself rather than being cut off by `head`: under `set -o pipefail`, head
  # closing the pipe early kills git with SIGPIPE, the pipeline exits 141 and `set -e` ends the
  # whole build with no message, as soon as more than 200 changes lie since the previous tag
  # (608 did on 24 September). A later `-n` in CHANGE_RANGE (the no-tag case) still wins.
  # shellcheck disable=SC2086
  git -C "$PROJECT_ROOT" log --no-merges --format='- %s' -n 200 $CHANGE_RANGE 2>/dev/null
} > "$ROOT/share/CHANGELOG.md"
echo "   + share/CHANGELOG.md ($(grep -c '^- ' "$ROOT/share/CHANGELOG.md") changes, $CHANGE_SINCE)"

# The same string, one line, no keys: `cat /opt/yantrik/.version` is what a person types, and
# `.version` existed before BUILD did so scripts and habits still point at it.
#
# It used to be written only by the ISO builder, from that script's own idea of the version, and
# by nothing afterwards — so a machine held `.version` = 0.3.0 (a literal that sat in the ISO
# script for five months) beside `BUILD` = version=v0.1.0-179-g6fc8b13, and answered whichever
# one you happened to ask. Both files come off this one variable now; the ISO build takes its
# copy out of the marker, and `yantrik-update` rewrites both on every apply and rollback.
printf '%s\n' "$VERSION" > "$ROOT/.version"

mkdir -p "$OUT_DIR"
TARBALL="$OUT_DIR/${NAME}.tar.zst"
say "Packing"
if command -v zstd >/dev/null 2>&1; then
  tar --zstd -cf "$TARBALL" -C "$STAGE" "$NAME"
else
  TARBALL="$OUT_DIR/${NAME}.tar.gz"
  tar -czf "$TARBALL" -C "$STAGE" "$NAME"
  echo "   zstd not installed — wrote gzip instead"
fi

# A checksum beside the artifact, because "did the download finish" and "is this the build
# I think it is" are the two questions every install path ends up asking.
( cd "$OUT_DIR" && sha256sum "$(basename "$TARBALL")" > "$(basename "$TARBALL").sha256" )

# Stable names, so cloud-init and the ISO can point at one URL forever.
case "$TARBALL" in *.tar.zst) LEXT=tar.zst ;; *.tar.gz) LEXT=tar.gz ;; *) LEXT="${TARBALL##*.}" ;; esac
ln -sf "$(basename "$TARBALL")" "$OUT_DIR/yantrik-os-linux-amd64.$LEXT" 2>/dev/null || true

say "Built"
echo "   $TARBALL"
echo "   $(du -h "$TARBALL" | cut -f1)  ·  $(cut -d= -f2 <<<"$(grep binaries "$ROOT/BUILD")") binaries  ·  $GITREV"

# ── Publishing ─────────────────────────────────────────────────────────────────────────
#
# Only runs with --publish CHANNEL. Kept in this script rather than a separate one because
# the artifact and its checksum are produced here, and a publisher that recomputes either
# can disagree with what was built — which is the kind of difference nobody notices until
# a machine installs something other than what was tested.
if [ -n "${PUBLISH_CHANNEL:-}" ]; then
  RELEASES_IP="${RELEASES_IP:-192.168.4.28}"
  SSH_KEY="${SSH_KEY:-$HOME/.ssh/id_deploy}"
  # IdentitiesOnly=yes so ssh offers this key and only this key. Without it, ssh tries every
  # key the agent holds first and the server can refuse the connection before the right one is
  # reached — the exact failure that made a homelab host look unreachable earlier in this work.
  SSH_OPTS="-o StrictHostKeyChecking=no -o IdentitiesOnly=yes -o BatchMode=yes -i $SSH_KEY"
  REMOTE="/var/www/releases/$PUBLISH_CHANNEL"

  say "Publishing to $PUBLISH_CHANNEL"
  BASE="$(basename "$TARBALL")"
  # `${BASE##*.}` strips only the LAST extension, so a .tar.zst published as .zst — a name
  # nothing asks for. The verification below passed anyway, because it checked the name it
  # had just written rather than the one a consumer uses.
  case "$BASE" in
    *.tar.zst) EXT="tar.zst" ;;
    *.tar.gz)  EXT="tar.gz" ;;
    *)         EXT="${BASE##*.}" ;;
  esac

  ssh $SSH_OPTS "root@$RELEASES_IP" "mkdir -p $REMOTE" || fail "cannot reach $RELEASES_IP"

  # Upload under the dated name first, then move the -latest pointer. A reader that catches
  # the window sees the old build rather than a half-written one.
  scp $SSH_OPTS "$TARBALL" "$TARBALL.sha256" "root@$RELEASES_IP:$REMOTE/" \
    || fail "upload failed"
  ssh $SSH_OPTS "root@$RELEASES_IP" \
    "cd $REMOTE && ln -sf '$BASE' 'yantrik-os-latest-linux-amd64.$EXT' && ln -sf '$BASE.sha256' 'yantrik-os-latest-linux-amd64.$EXT.sha256'" \
    || fail "could not update the latest pointer"

  # The manifest is what a machine reads to answer "is there something newer than me".
  # Updated in place so the other channels keep whatever they were pointing at.
  ssh $SSH_OPTS "root@$RELEASES_IP" "python3 - <<'PYEOF'
import json, os
path = '/var/www/releases/manifest.json'
m = {'channels': {}}
if os.path.exists(path):
    try:
        m = json.load(open(path))
    except Exception:
        pass   # a corrupt manifest should not stop a good build being published
m.setdefault('channels', {})['$PUBLISH_CHANNEL'] = {
    'version': '$VERSION',
    'date': '$(date -u +%Y-%m-%d)',
    'url': '/$PUBLISH_CHANNEL',
    'notes': 'Build $GITREV ($(date -u +%Y-%m-%d))',
    'git': '$GITREV',
    'artifact': '$BASE',
    'sha256': '$(cut -d" " -f1 < "$TARBALL.sha256")',
    'binaries': ${#BINS[@]},
}
json.dump(m, open(path, 'w'), indent=2)
print('  manifest: $PUBLISH_CHANNEL -> $VERSION')
PYEOF" || fail "manifest update failed"

  # Verify by fetching, not by trusting the upload. The checksum is the whole point of
  # publishing one: a 200 with the wrong bytes reads exactly like a 200 with the right ones.
  say "Verifying what is actually being served"
  #
  # Fetch the channel THIS RUN published to. That sounds obvious; it was not what happened.
  #
  # This used to read the URL out of cloud-init/user-data.yaml, on the reasoning that a
  # publisher and an installer which each derive the name separately will drift apart. The
  # reasoning is right and the implementation was wrong: that file names one specific channel
  # (nightly), so publishing to any other channel downloaded nightly's bundle and compared it
  # against the bundle we had just built somewhere else. A correct publish to `stable` failed
  # verification with a hash that, read against the manifest, turned out to be nightly's.
  #
  # So the URL comes from the channel, and cloud-init is used for what it can actually
  # settle — the host — with a warning rather than a failure if it points somewhere else.
  CI_URL="$(grep -o 'http[^"]*yantrik-os-latest[^"]*' "$SCRIPT_DIR/cloud-init/user-data.yaml" 2>/dev/null | head -1)"
  HOST_URL="$(printf '%s' "$CI_URL" | sed -n 's#^\(https\?://[^/]*\)/.*#\1#p')"
  URL="${HOST_URL:-http://releases.yantrikos.com}/$PUBLISH_CHANNEL/yantrik-os-latest-linux-amd64.$EXT"

  CI_CHANNEL="$(printf '%s' "$CI_URL" | sed -n 's#.*/\([^/]*\)/yantrik-os-latest.*#\1#p')"
  if [ -n "$CI_CHANNEL" ] && [ "$CI_CHANNEL" != "$PUBLISH_CHANNEL" ]; then
    printf '   note: a fresh install follows %s, and this build went to %s\n' \
      "$CI_CHANNEL" "$PUBLISH_CHANNEL"
  fi

  GOT="$(curl -sfL "$URL" | sha256sum | cut -d' ' -f1)" || fail "cannot fetch $URL"
  WANT="$(cut -d' ' -f1 < "$TARBALL.sha256")"
  if [ "$GOT" = "$WANT" ]; then
    echo "   $URL"
    echo "   sha256 matches the artifact that was built"
  else
    fail "served bytes do not match what was built (got $GOT, want $WANT)"
  fi

  # ── Retention ──
  #
  # A nightly channel with no retention is a disk filling at ~240 MB a build; five had
  # accumulated to 1.4 GB before anyone looked. Keep the newest RETAIN bundles and delete the
  # rest, AFTER the verification above — so a publish that turned out to serve the wrong bytes
  # has not already deleted the build that was working.
  #
  # Deliberately by modification time and never by name: the version string contains a date
  # that is the BUILD date, and a rebuild of an old commit would sort itself into the wrong
  # place. Whatever `-latest` points at is protected regardless of age.
  RETAIN="${RELEASE_RETAIN:-3}"
  say "Pruning $PUBLISH_CHANNEL to the newest $RETAIN"
  ssh $SSH_OPTS "root@$RELEASES_IP" "
    cd $REMOTE || exit 0
    KEEP=\$(readlink yantrik-os-latest-linux-amd64.$EXT 2>/dev/null)
    ls -1t *.tar.zst *.tar.gz 2>/dev/null | grep -v '^yantrik-os-latest' | tail -n +\$(($RETAIN + 1)) | while read -r old; do
      [ \"\$old\" = \"\$KEEP\" ] && continue
      rm -f -- \"\$old\" \"\$old.sha256\"
      echo \"   removed \$old\"
    done
    echo \"   \$(ls -1 *.tar.zst *.tar.gz 2>/dev/null | grep -v '^yantrik-os-latest' | wc -l) kept, \$(df -h . | awk 'NR==2{print \$4}') free\"
  " || echo "   (prune skipped)"
fi
