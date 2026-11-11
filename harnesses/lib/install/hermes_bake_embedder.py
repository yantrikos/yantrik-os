"""Fetch the embedder a new Hermes memory opens with, into this account's cache: hermes_bake_embedder.py MEMORY_DIR

Run by hermes-bake.sh while the image is built, with the image's own Python and the engine at
MEMORY_DIR/mcp (/opt/yantrik/memory, bake-memory.sh). A new file-backed YantrikDB store is created
on the engine's default embedder, which the engine downloads once from
github.com/yantrikos/yantrikdb-models into `$HOME/.cache/yantrikdb/models`, checked against the
SHA-256 pinned in the engine. Without it in the image a first run needs the network for that, and
offline the engine quietly opens the store on its small bundled model instead, which a store keeps
for good (its dimension is fixed when it is made).

So this does what the first run's `with_default` does, with the network, once: the engine's own
fetch and its own pin, nothing re-implemented here. The licence gate (hermes_licences.py) then reads
the cache and stops the build unless the model it needs is there. Prints nothing; exits 1 with the
engine's words when it cannot open a store.
"""

import os
import sys
import tempfile

if len(sys.argv) != 2:
    print("usage: hermes_bake_embedder.py MEMORY_DIR", file=sys.stderr)
    sys.exit(2)
sys.path.insert(0, os.path.join(sys.argv[1], "mcp"))
from yantrikdb import YantrikDB  # noqa: E402

with tempfile.TemporaryDirectory() as scratch:
    store = YantrikDB.with_default(os.path.join(scratch, "embedder-probe.db"))
    store.close()
