"""Make sure Hermes's memory can embed before an install says it can remember: hermes_embedder.py

Run by hermes.sh with Hermes's own Python, after the YantrikDB provider is installed and Hermes's
`.env` is written. The engine downloads a named embedder (potion-base-8M and the like) once, into
`$XDG_CACHE_HOME/yantrikdb/models` or `$HOME/.cache/yantrikdb/models`, and raises "embedder not
configured" when it is not there and cannot be fetched. Under the mind account (#411) that cache is
the account's own and starts empty: a Hermes moved from a person's home brings theirs along
(harness-isolation.sh), and a first install has to fetch it, through the egress proxy. This does that
fetch now, with the same settings the provider will use (the YANTRIKDB_* lines of Hermes's .env), so
that a failure is named here, at the step that can say why, and not as a failed proof afterwards.

It writes nothing to Hermes's memory: opening the backend and a recall attach the embedder, and that
is all. Prints one line; exits 0 when the embedder is ready. Never prints an environment value.
"""

import os
import sys

home = os.path.expanduser(os.environ.get("HERMES_HOME", "~/.hermes"))
# Where the install put the provider (hermes_runtime.VENDOR), last, as the shim Hermes loads does.
sys.path.append(os.path.join(home, "plugins", "yantrikdb", "vendor"))
try:
    for line in open(os.path.join(home, ".env"), encoding="utf-8"):
        key, sep, value = line.strip().partition("=")
        if sep and key.startswith("YANTRIKDB_"):
            os.environ[key] = value.strip().strip('"').strip("'")
except OSError:
    pass

try:
    from yantrikdb_hermes_plugin.client import YantrikDBConfig
    from yantrikdb_hermes_plugin.embedded import make_backend

    config = YantrikDBConfig.from_env()
    client = make_backend(config)
    client.recall("embedder check", top_k=1)
except Exception as exc:  # the engine's own words; one line, bounded, no environment in it
    text = " ".join(str(exc).split())[:300] or exc.__class__.__name__
    print("the embedder is not ready: %s" % text)
    sys.exit(1)
print("embedder ready (%s)" % (getattr(config, "embedder_name", "") or "bundled"))
