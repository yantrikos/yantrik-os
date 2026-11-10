"""Whether this process has the workshops folder hidden, as the surface needs to be sure of.

`/var/lib/yantrik/workshops` holds every Mind's private workshop. A program the person
launches is given a tmpfs over it (the sealed workshop), so nothing in it can see another
account's or a Mind's folder. Blender holds a control surface that can run Python, so it keeps
its own invariant: when the folder exists and this process shows no tmpfs over it, the socket
is not bound. A launcher that forgot the mask therefore cannot hand out the surface.
Blender itself still opens, and says why on stderr.
"""

WORKSHOPS = "/var/lib/yantrik/workshops"
MOUNTINFO = "/proc/self/mountinfo"


def _unescape(field):
    """mountinfo writes space, tab, newline and backslash as octal escapes."""
    out, i = [], 0
    while i < len(field):
        if field[i] == "\\" and field[i + 1:i + 4].isdigit():
            out.append(chr(int(field[i + 1:i + 4], 8)))
            i += 4
        else:
            out.append(field[i])
            i += 1
    return "".join(out)


def tmpfs_over(mountinfo_text, path):
    """True when a tmpfs is mounted exactly at `path` (the last mount at it wins)."""
    covered = False
    for line in mountinfo_text.splitlines():
        head, sep, tail = line.partition(" - ")
        fields = head.split()
        if not sep or len(fields) < 5:
            continue
        if _unescape(fields[4]) == path:
            covered = tail.split()[:1] == ["tmpfs"]
    return covered


def refusal(workshops=WORKSHOPS, mountinfo=MOUNTINFO, exists=None):
    """Why the surface must not be bound, or None when it may be.

    Nothing to hide when the folder is absent. A mountinfo that cannot be read is
    treated as not masked, because the folder is there and nothing shows it hidden.
    """
    import os
    if not (exists or os.path.isdir)(workshops):
        return None
    try:
        with open(mountinfo, encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError:
        text = ""
    if tmpfs_over(text, workshops):
        return None
    return ("control surface not bound: %s exists and this process has no tmpfs over it "
            "(per %s), so it was not started in its sealed view; open Blender from the "
            "dock or the app launcher" % (workshops, mountinfo))
