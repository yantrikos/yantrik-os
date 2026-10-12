#!/bin/sh
# Bake YantrikDB, every mind's memory (#677), into a directory: bake-memory.sh DEST | --check DIR
#
# The one place that decides what /opt/yantrik/memory holds. The release bundle carries it
# (build-release.sh → memory/), the updater installs it (yantrik-update apply), and the ISO takes
# it from the bundle it unpacks (build-debian-iso.sh), so a machine set up by update and one set up
# from an image have the same memory, byte for byte. Before this the ISO baked it alone, and a
# machine that got the build by update had no /opt/yantrik/memory: every harness install stopped at
# "this image has no yantrikdb-mcp" (VM 520, build 1448).
#
# Offline at the other end: a harness installed from Settings gets YantrikDB as its memory with
# nothing fetched from a catalog, git or PyPI, and with no terminal (the Mac failure of #677 was the
# provider's dependencies refused by a non-interactive install).
#
#   wheels/   the engine and the Hermes provider, installed by harnesses/lib/install/hermes.sh
#             into Hermes's own Python with --no-index. Hermes picks its Python itself (3.14 at
#             the commit hermes.sh pins), so the one compiled dependency is fetched for every
#             CPython it may be; the engine is abi3 and the rest pure Python.
#   mcp/      yantrikdb-mcp and its dependencies for the image's own python3, which is how Pi,
#   bin/      DeepSeek and OpenClaw reach YantrikDB (bin/yantrikdb-mcp starts it, with -I so neither
#             the person's own site-packages nor the directory it runs in can shadow it).
#   VERSIONS  what is in it, and which Python mcp/ is for.
#
# The provider is yantrikdb-hermes-plugin with the shared `yantrik` mode (yantrikdb-hermes-plugin
# PR #93), by commit: a tag can move and a commit cannot. Its mode stays `embedded` until #586's
# tamper-proof door lands (hermes.sh says why), and switching then needs nothing new here.
# Moved on by hand, together, with a review of what changed.
#
# YANTRIK_IMAGE_PYTHON is the image's python3 (Debian trixie: 3.13); the ISO checks its chroot
# agrees and the updater warns on a machine whose python3 does not. YANTRIK_ARCH is amd64 or arm64.
# Needs python3 with pip, and git, on the machine that builds.
set -eu

fail() { echo "bake-memory: $*" >&2; exit 1; }

# check DIR — DIR holds everything a harness install reads from /opt/yantrik/memory, or this says
# what is missing. `bake-memory.sh --check DIR` runs only this: the release build checks the bundle
# with it, the ISO checks what it copied, and the updater's selftest checks the bundle's file list.
check() {
    d=$1
    [ -f "$d/VERSIONS" ] || fail "$d has no VERSIONS"
    [ -x "$d/bin/yantrikdb-mcp" ] || fail "$d has no executable bin/yantrikdb-mcp"
    grep -q -- ' -I -c ' "$d/bin/yantrikdb-mcp" || fail "$d/bin/yantrikdb-mcp does not start Python with -I"
    ls "$d/wheels/"yantrikdb-[0-9]*.whl >/dev/null 2>&1 || fail "$d/wheels has no YantrikDB engine wheel"
    ls "$d/wheels/"yantrikdb_hermes_plugin-*.whl >/dev/null 2>&1 || fail "$d/wheels has no Hermes provider wheel"
    ls "$d/wheels/"uuid_utils-*.whl >/dev/null 2>&1 || fail "$d/wheels has no uuid-utils wheel"
    ls "$d/wheels/"click-*.whl >/dev/null 2>&1 || fail "$d/wheels has no click wheel"
    [ -d "$d/mcp/yantrikdb_mcp" ] || fail "$d/mcp has no yantrikdb_mcp"
    [ -d "$d/mcp/yantrikdb" ] || fail "$d/mcp has no yantrikdb engine"
    [ -z "$(find "$d" ! -type l -perm /022 2>/dev/null | head -n 1)" ] || fail "$d has files others can write"
}

if [ "${1:-}" = --check ]; then
    [ $# -eq 2 ] || { echo "usage: bake-memory.sh --check DIR" >&2; exit 2; }
    check "$2"
    exit 0
fi
[ $# -eq 1 ] || { echo "usage: bake-memory.sh DEST | --check DIR" >&2; exit 2; }
dest=$1

YANTRIKDB_VERSION=0.23.2
YANTRIKDB_MCP_VERSION=0.24.0
YANTRIKDB_HERMES_PLUGIN_COMMIT=d1f82fab223bc092557f56c3d2ff39f6ca65c583
YANTRIKDB_DEPS="uuid-utils==1.0.0 click==8.3.1"
IMAGE_PY=${YANTRIK_IMAGE_PYTHON:-3.13}
ARCH=${YANTRIK_ARCH:-amd64}

case "$ARCH" in
    amd64) PY_PLATFORMS="--platform manylinux_2_17_x86_64 --platform manylinux2014_x86_64 --platform manylinux_2_28_x86_64 --platform linux_x86_64" ;;
    arm64) PY_PLATFORMS="--platform manylinux_2_17_aarch64 --platform manylinux2014_aarch64 --platform manylinux_2_28_aarch64 --platform linux_aarch64" ;;
    *) fail "no YantrikDB wheels for $ARCH" ;;
esac
python3 -m pip --version >/dev/null 2>&1 || fail "python3 has no pip here; install python3-pip"
command -v git >/dev/null 2>&1 || fail "git is needed to build the Hermes provider at its commit"

rm -rf "$dest"
mkdir -p "$dest/wheels" "$dest/bin" || fail "cannot make $dest"
for py in 3.11 3.12 3.13 3.14; do
    # shellcheck disable=SC2086  # PY_PLATFORMS and YANTRIKDB_DEPS are word lists on purpose
    python3 -m pip download --quiet --disable-pip-version-check --dest "$dest/wheels" \
        --only-binary=:all: --no-deps --implementation cp --python-version "$py" $PY_PLATFORMS \
        "yantrikdb==$YANTRIKDB_VERSION" $YANTRIKDB_DEPS \
        || fail "could not fetch the YantrikDB wheels for CPython $py"
done
python3 -m pip wheel --quiet --disable-pip-version-check --no-deps --wheel-dir "$dest/wheels" \
    "git+https://github.com/yantrikos/yantrikdb-hermes-plugin@$YANTRIKDB_HERMES_PLUGIN_COMMIT" \
    || fail "could not build the YantrikDB Hermes provider at $YANTRIKDB_HERMES_PLUGIN_COMMIT"
# shellcheck disable=SC2086
python3 -m pip install --quiet --disable-pip-version-check --target "$dest/mcp" \
    --only-binary=:all: --implementation cp --python-version "$IMAGE_PY" $PY_PLATFORMS \
    "yantrikdb-mcp==$YANTRIKDB_MCP_VERSION" "yantrikdb==$YANTRIKDB_VERSION" \
    || fail "could not fetch yantrikdb-mcp $YANTRIKDB_MCP_VERSION for Python $IMAGE_PY"
rm -rf "$dest/mcp/bin"
find "$dest" -name __pycache__ -type d -prune -exec rm -rf {} +
cat > "$dest/bin/yantrikdb-mcp" <<'MCP'
#!/bin/sh
# YantrikDB's MCP server on the image's own Python, from /opt/yantrik/memory/mcp (#677).
# -I, not -s: with -c, Python puts the current directory on sys.path, and this runs in whatever
# directory the harness is working in (a project Pi was opened in, OpenClaw's workspace), where a
# json.py or typing.py a mind's graded tools wrote would run here ungraded. -I leaves out the
# current directory, the person's site-packages and every PYTHON* variable.
exec /usr/bin/python3 -I -c 'import sys; sys.path.insert(0, "/opt/yantrik/memory/mcp"); from yantrikdb_mcp import main; sys.exit(main())' "$@"
MCP
chmod 755 "$dest/bin/yantrikdb-mcp"
chmod -R go-w "$dest"
{
    echo "yantrikdb=$YANTRIKDB_VERSION"
    echo "yantrikdb-mcp=$YANTRIKDB_MCP_VERSION"
    echo "yantrikdb-hermes-plugin=$YANTRIKDB_HERMES_PLUGIN_COMMIT"
    echo "image-python=$IMAGE_PY"
    echo "arch=$ARCH"
    (cd "$dest/wheels" && ls -1)
} > "$dest/VERSIONS"

check "$dest"
echo "bake-memory: $dest holds engine $YANTRIKDB_VERSION, the Hermes provider and yantrikdb-mcp $YANTRIKDB_MCP_VERSION for Python $IMAGE_PY"
