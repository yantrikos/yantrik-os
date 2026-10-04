#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# setup-arm64-cross.sh — make this x86_64 machine build the workspace for arm64
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# #270's Pi 5 image and general ARM64 image both need every binary in the workspace built
# for aarch64-unknown-linux-gnu. That build worked by hand on a Debian 13 machine: the
# cross gcc, the arm64 copies of every library the -sys crates link against, and six
# environment variables telling cargo and pkg-config where all of it lives. This script is
# the recipe made repeatable — idempotent, Debian 13 and Ubuntu 24.04 both — and the CI
# arm64 job runs it, so the way an image gets built and the way CI proves the build green
# cannot drift apart.
#
# The environment lives in arm64-cross.env beside this script: one copy, which this script
# prints at the end and CI appends to $GITHUB_ENV.
#
# Ubuntu needs apt work Debian does not. arm64 packages are not on the amd64 mirrors; they
# come from ports.ubuntu.com. And enabling the arm64 architecture without restricting the
# existing sources to amd64 makes apt-get update ask every mirror for arm64 indexes it does
# not have, which fails the update outright. So on Ubuntu every existing entry is
# restricted to the host architecture and one ports entry restricted to arm64 is added.
# Debian carries arm64 as a release architecture on the same mirrors as amd64, so there
# `dpkg --add-architecture arm64` is the whole of the sources work.
#
#   sudo deploy/yantrik-os/setup-arm64-cross.sh        # set this machine up
#   deploy/yantrik-os/setup-arm64-cross.sh selftest    # offline checks, no root, no apt
#
# The script does not own anybody's rustup: after it, the building user still runs
# `rustup target add aarch64-unknown-linux-gnu` (CI does that itself).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENV_FILE="$SCRIPT_DIR/arm64-cross.env"

# The cross toolchain, host architecture. clang is here because build scripts that
# generate bindings compile against libclang, and they stop the cross build even when
# everything else links.
CROSS_TOOLS=(gcc-aarch64-linux-gnu g++-aarch64-linux-gnu libc6-dev-arm64-cross
             pkg-config clang cmake)

# The arm64 copies of every native library the workspace links against: alsa, udev,
# openssl, speech-dispatcher, wayland, and the x11/xcb/xkbcommon/fontconfig/freetype set
# Slint's winit backend needs. The same list the hand build on Debian 13 was verified with.
ARM64_LIBS=(libwayland-dev libxkbcommon-dev libfontconfig1-dev libfreetype-dev
            libx11-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev
            libxcb-xfixes0-dev libgl-dev libasound2-dev libssl-dev libudev-dev
            libdbus-1-dev zlib1g-dev libspeechd-dev)

# ── apt sources rewriting (pure: stdin to stdout, so the selftest can hold them) ──────

# deb822 sources — the format Debian 13 and Ubuntu 24.04 both use — with every stanza
# restricted to the given architecture. A stanza that already declares Architectures is
# passed through untouched, so a second run reprints its input.
restrict_deb822() {
  awk -v arch="$1" '
    BEGIN { RS = ""; FS = "\n" }                       # one stanza per record
    {
      has_types = 0; has_arch = 0
      for (i = 1; i <= NF; i++) {
        if ($i ~ /^Types:/)         has_types = 1
        if ($i ~ /^Architectures:/) has_arch  = 1
      }
      for (i = 1; i <= NF; i++) {
        printf "%s\n", $i
        if (has_types && !has_arch && $i ~ /^Types:/) printf "Architectures: %s\n", arch
      }
      printf "\n"
    }
  '
}

# One-line sources (`deb URI suite components…`) with [arch=…] added to every line that
# carries no options group. A line that already has a group is left as its author wrote it
# rather than merged: the format allows one group per line, and guessing how to rewrite
# somebody's signed-by is worse than leaving it.
restrict_oneline() {
  sed -E "/^(deb|deb-src)[[:space:]]+\[/! s/^(deb|deb-src)[[:space:]]+/\1 [arch=$1] /"
}

# The arm64 half of Ubuntu's sources: the release, updates and security pockets of the
# ports mirror, restricted to arm64. The same content on every run, so writing it is
# repeatable by construction.
ubuntu_ports_sources() {
  local pocket
  for pocket in "" "-updates" "-security"; do
    printf 'deb [arch=arm64] http://ports.ubuntu.com/ubuntu-ports %s%s main universe\n' "$1" "$pocket"
  done
}

# Run a rewriter over a file and touch the file only if the result differs — apt notices
# mtimes, and so does whoever reads an idempotent script's output twice.
restrict_file() {
  local rewriter="$1" file="$2" arch="$3" tmp
  tmp="$(mktemp)"
  "$rewriter" "$arch" < "$file" > "$tmp"
  if ! cmp -s "$tmp" "$file"; then
    cat "$tmp" > "$file"        # cat, not mv: the file keeps its owner and mode
    echo "  $file: restricted to $arch"
  fi
  rm -f "$tmp"
}

# ── setup ─────────────────────────────────────────────────────────────────────────────

cmd_setup() {
  if [ "$(id -u)" -ne 0 ]; then
    echo "setup-arm64-cross.sh: needs root for dpkg and apt — sudo $0" >&2
    exit 1
  fi
  if [ ! -f "$ENV_FILE" ]; then
    echo "setup-arm64-cross.sh: $ENV_FILE is missing — the script and its env file ship together" >&2
    exit 1
  fi

  local host_arch distro codename
  host_arch="$(dpkg --print-architecture)"
  if [ "$host_arch" != "amd64" ]; then
    echo "setup-arm64-cross.sh: this machine is $host_arch. The script sets up an x86_64" >&2
    echo "  machine to cross-build for arm64; on an arm64 machine the build is native and" >&2
    echo "  none of this applies." >&2
    exit 1
  fi
  # shellcheck source=/dev/null
  . /etc/os-release
  codename="${VERSION_CODENAME:-}"
  case " ${ID:-} ${ID_LIKE:-} " in
    *ubuntu*) distro=ubuntu ;;
    *debian*) distro=debian ;;
    *)
      echo "setup-arm64-cross.sh: ${ID:-this system} is neither Debian nor Ubuntu — the apt" >&2
      echo "  work here is written for those two and guesses nowhere else." >&2
      exit 1
      ;;
  esac

  echo "[1/4] Enabling the arm64 architecture..."
  if dpkg --print-foreign-architectures | grep -qx arm64; then
    echo "  already enabled"
  else
    dpkg --add-architecture arm64
    echo "  arm64 added"
  fi

  if [ "$distro" = ubuntu ]; then
    echo "[2/4] apt sources: amd64 stays on the existing mirrors, arm64 comes from ports.ubuntu.com..."
    if [ -z "$codename" ]; then
      echo "setup-arm64-cross.sh: /etc/os-release has no VERSION_CODENAME to name the ports suites with" >&2
      exit 1
    fi
    # Every source file on the machine, not just Ubuntu's own: with arm64 enabled, any
    # amd64-only third-party repo without an arch restriction fails the update the same way.
    local f ports_file tmp
    shopt -s nullglob
    for f in /etc/apt/sources.list.d/*.sources; do restrict_file restrict_deb822 "$f" "$host_arch"; done
    for f in /etc/apt/sources.list.d/*.list;    do restrict_file restrict_oneline "$f" "$host_arch"; done
    shopt -u nullglob
    if [ -s /etc/apt/sources.list ]; then
      restrict_file restrict_oneline /etc/apt/sources.list "$host_arch"
    fi

    ports_file=/etc/apt/sources.list.d/arm64-ports.list
    tmp="$(mktemp)"
    ubuntu_ports_sources "$codename" > "$tmp"
    if ! cmp -s "$tmp" "$ports_file" 2>/dev/null; then
      cat "$tmp" > "$ports_file"
      chmod 0644 "$ports_file"
      echo "  $ports_file: arm64 from ports.ubuntu.com ($codename)"
    fi
    rm -f "$tmp"
  else
    echo "[2/4] Debian serves arm64 from the same mirrors as amd64 — no sources work."
  fi

  echo "[3/4] Installing the cross toolchain..."
  apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${CROSS_TOOLS[@]}"

  echo "[4/4] Installing the arm64 libraries the workspace links against..."
  local libs=("${ARM64_LIBS[@]/%/:arm64}")
  DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${libs[@]}"

  echo
  echo "Done. The environment a build needs, from $ENV_FILE:"
  sed 's/^/  /' "$ENV_FILE"
  echo
  echo "Then, as the user that builds:"
  echo "  rustup target add aarch64-unknown-linux-gnu"
  echo "  set -a; . deploy/yantrik-os/arm64-cross.env; set +a"
  echo "  cargo build --release --target aarch64-unknown-linux-gnu --workspace --bins"
}

# ── selftest ──────────────────────────────────────────────────────────────────────────
#
# No root, no apt, no network: the sources rewriters are pure text transforms and the env
# file is text, so everything a machine could get wrong before `apt-get update` is held
# here. What this cannot prove is that apt and the cross gcc agree afterwards; that is the
# CI arm64 job's half.
cmd_selftest() {
  local fails=0
  local c_grn c_red c_off
  c_grn="$(printf '\033[0;32m')"; c_red="$(printf '\033[0;31m')"; c_off="$(printf '\033[0m')"

  t_ok()   { printf '  %sok%s   %s\n' "$c_grn" "$c_off" "$1"; }
  t_fail() { printf '  %sFAIL%s %s\n' "$c_red" "$c_off" "$1"; fails=$((fails + 1)); }

  echo "setup-arm64-cross.sh selftest"

  local fixture out out2 n

  # 1. The deb822 rewriter, against sources shaped the way an Ubuntu 24.04 machine
  #    (including the CI runner) ships them: two stanzas, neither restricted. Both must
  #    come out with Architectures: amd64 and keep every field they had.
  fixture='Types: deb
URIs: http://azure.archive.ubuntu.com/ubuntu/
Suites: noble noble-updates noble-backports
Components: main universe restricted multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg

Types: deb
URIs: http://security.ubuntu.com/ubuntu/
Suites: noble-security
Components: main universe restricted multiverse
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg'
  out="$(printf '%s\n' "$fixture" | restrict_deb822 amd64)"
  n="$(printf '%s\n' "$out" | grep -c '^Architectures: amd64$' || true)"
  if [ "$n" -eq 2 ]; then
    t_ok "deb822: both stanzas came out restricted to amd64"
  else
    t_fail "deb822: expected 2 'Architectures: amd64' lines, found $n"
  fi
  if printf '%s\n' "$out" | grep -q '^URIs: http://azure.archive.ubuntu.com/ubuntu/$' \
     && printf '%s\n' "$out" | grep -q '^Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg$'; then
    t_ok "deb822: the stanzas' own fields came through unchanged"
  else
    t_fail "deb822: lost or altered a field of the original stanzas"
  fi

  # 2. Idempotent, the property the whole script is promised on: a second pass over its
  #    own output must be a fixed point, or a re-run of the setup rewrites sources again.
  out2="$(printf '%s\n' "$out" | restrict_deb822 amd64)"
  if [ "$out2" = "$out" ]; then
    t_ok "deb822: a second pass changed nothing"
  else
    t_fail "deb822: a second pass changed the file — not idempotent"
  fi

  # 3. A stanza that already declares its architectures is the ports stanza of a machine
  #    this script has already run on; it must survive verbatim, not gain a second line.
  fixture='Types: deb
URIs: http://ports.ubuntu.com/ubuntu-ports
Architectures: arm64
Suites: noble
Components: main universe'
  out="$(printf '%s\n' "$fixture" | restrict_deb822 amd64)"
  n="$(printf '%s\n' "$out" | grep -c '^Architectures:' || true)"
  if [ "$n" -eq 1 ] && printf '%s\n' "$out" | grep -q '^Architectures: arm64$'; then
    t_ok "deb822: an already-restricted stanza kept its own Architectures line"
  else
    t_fail "deb822: an already-restricted stanza came out with $n Architectures lines"
  fi

  # 4. The one-line rewriter, against sources shaped the way older Ubuntu and hand-built
  #    machines have them: a plain line, a comment, and a line that already carries an
  #    options group.
  fixture='deb http://archive.ubuntu.com/ubuntu noble main restricted universe multiverse
deb-src http://archive.ubuntu.com/ubuntu noble main
# deb http://example.com disabled
deb [signed-by=/usr/share/keyrings/x.gpg] http://dl.example.com/pkg stable main'
  out="$(printf '%s\n' "$fixture" | restrict_oneline amd64)"
  if printf '%s\n' "$out" | sed -n 1p | grep -qx 'deb \[arch=amd64\] http://archive.ubuntu.com/ubuntu noble main restricted universe multiverse' \
     && printf '%s\n' "$out" | sed -n 2p | grep -qx 'deb-src \[arch=amd64\] http://archive.ubuntu.com/ubuntu noble main'; then
    t_ok "one-line: plain deb and deb-src lines came out restricted to amd64"
  else
    t_fail "one-line: a plain line was not restricted correctly:"
    printf '%s\n' "$out" | sed -n '1,2p' | sed 's/^/      /'
  fi
  if printf '%s\n' "$out" | sed -n 3p | grep -qx '# deb http://example.com disabled' \
     && printf '%s\n' "$out" | sed -n 4p | grep -qx 'deb \[signed-by=/usr/share/keyrings/x.gpg\] http://dl.example.com/pkg stable main'; then
    t_ok "one-line: comments and lines with an options group were left alone"
  else
    t_fail "one-line: touched a comment or a line that already had an options group"
  fi
  out2="$(printf '%s\n' "$out" | restrict_oneline amd64)"
  if [ "$out2" = "$out" ]; then
    t_ok "one-line: a second pass changed nothing"
  else
    t_fail "one-line: a second pass changed the file — not idempotent"
  fi

  # 5. The ports entry: three pockets, all arm64-only, all from the ports mirror — an
  #    arm64 index fetched from archive.ubuntu.com is a 404 that fails apt-get update.
  out="$(ubuntu_ports_sources noble)"
  n="$(printf '%s\n' "$out" | wc -l)"
  if [ "$n" -eq 3 ] \
     && [ "$(printf '%s\n' "$out" | grep -c '^deb \[arch=arm64\] http://ports.ubuntu.com/ubuntu-ports noble\(-updates\|-security\)\? main universe$' || true)" -eq 3 ]; then
    t_ok "ports: release, updates and security pockets, arm64-only, from ports.ubuntu.com"
  else
    t_fail "ports: expected the three noble pockets, got:"
    printf '%s\n' "$out" | sed 's/^/      /'
  fi

  # 6. The env file: every non-comment line must be KEY=value — bash sources it and CI
  #    appends it to $GITHUB_ENV, and one stray word breaks both readers — and the six
  #    variables a cross build needs must be in it.
  if [ ! -f "$ENV_FILE" ]; then
    t_fail "$ENV_FILE is missing — the script and its env file ship together"
  else
    local line key missing=0
    while IFS= read -r line; do
      case "$line" in ''|'#'*) continue ;; esac
      if ! printf '%s\n' "$line" | grep -Eq '^[A-Za-z_][A-Za-z0-9_]*=.'; then
        t_fail "arm64-cross.env: '$line' is not a KEY=value line — unsourceable and unappendable"
        missing=1
      fi
    done < "$ENV_FILE"
    for key in CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER \
               CC_aarch64_unknown_linux_gnu CXX_aarch64_unknown_linux_gnu \
               PKG_CONFIG_ALLOW_CROSS PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR; do
      if ! grep -q "^$key=" "$ENV_FILE"; then
        t_fail "arm64-cross.env: $key is missing — a cross build without it finds the host's tools"
        missing=1
      fi
    done
    if [ "$missing" -eq 0 ]; then
      t_ok "arm64-cross.env: KEY=value throughout, and all six cross-build variables present"
    fi
  fi

  # 7. The CI workflow asks this script and this env file instead of retyping either. A
  #    second copy of the six variables in the workflow is exactly the drift the single
  #    file exists to prevent.
  local ci="$PROJECT_ROOT/.github/workflows/ci.yml"
  if [ -f "$ci" ]; then
    if grep -q 'setup-arm64-cross.sh' "$ci"; then
      t_ok "ci.yml runs setup-arm64-cross.sh"
    else
      t_fail "ci.yml no longer runs setup-arm64-cross.sh — CI and a real machine would set up differently"
    fi
    if grep -q 'arm64-cross.env' "$ci"; then
      t_ok "ci.yml reads arm64-cross.env"
    else
      t_fail "ci.yml no longer reads arm64-cross.env — the six variables have a second copy somewhere"
    fi
    if grep -q 'CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER' "$ci"; then
      t_fail "ci.yml writes the cross linker down by hand — that variable belongs in arm64-cross.env"
    else
      t_ok "ci.yml writes none of the env file's variables down by hand"
    fi
  else
    t_fail "$ci is missing — the arm64 job has nothing to be held against"
  fi

  if [ "$fails" -gt 0 ]; then
    printf '%s%s selftest check(s) failed%s\n' "$c_red" "$fails" "$c_off"
    return 1
  fi
  printf '%sall selftest checks passed%s\n' "$c_grn" "$c_off"
}

case "${1:-}" in
  selftest)  cmd_selftest ;;
  "")        cmd_setup ;;
  -h|--help) sed -n '2,29p' "$0" ;;
  *)
    echo "usage: setup-arm64-cross.sh [selftest]" >&2
    exit 2
    ;;
esac
