"""The clean-install arm: a machine whose memory holds no earlier tests' fixtures.

541's memory holds family fixtures (a sister's birthday, a city) left by earlier tests, so what a mind recalls or
could leak there is not what a fresh person's machine would hold. `--arm clean` refuses unless the memory stores look
fresh, and every results file records its arm, so the two kinds of run are never mixed in a comparison."""
import os

FIXTURE_WORDS = [b"Priya", b"Bentonville", b"Dallas", b"Arjun", b"Rohan", b"AX-4471"]
MAX_FRESH_BYTES = 3 * 1024 * 1024


def stores(machine, mind):
    if mind == "mind":
        return ["/var/lib/yantrik-mind/mind.db"]
    return [f"{machine.home}/.hermes/state.db"]


def fixtures_present(machine, mind):
    """[(path, reason)] for each store that is not fresh. Empty means the arm 'clean' is honest on this machine."""
    out = []
    for path in stores(machine, mind):
        try:
            size = os.path.getsize(path)
            with open(path, "rb") as f:
                blob = f.read(64 * 1024 * 1024)
        except OSError as e:
            out.append((path, f"unreadable: {e}"))
            continue
        found = [w.decode() for w in FIXTURE_WORDS if w in blob]
        if found:
            out.append((path, "holds fixtures: " + ", ".join(found)))
        elif size > MAX_FRESH_BYTES:
            out.append((path, f"is {size // 1024} KB, more than a fresh install's"))
    return out
