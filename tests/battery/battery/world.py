"""The sample world ("Alex"): snapshot it once, put it back before every run. Nothing is ever deleted:
what a run added or changed is moved aside under <state>/<stamp>/moved/ and listed."""
import hashlib
import os
import shutil
import time

STATE = "/var/lib/yantrik-battery"


def tracked(home):
    """label -> directory the battery watches. Order matters only for the report."""
    return {"desktop": f"{home}/Desktop",
            "notes": f"{home}/.local/share/yantrik/notes",
            "calendar": f"{home}/.local/share/yantrik/calendar"}


def _digest(path):
    h = hashlib.sha256()
    if os.path.isdir(path):
        for root, dirs, files in sorted(os.walk(path)):
            dirs.sort()
            for f in sorted(files):
                p = os.path.join(root, f)
                h.update(os.path.relpath(p, path).encode())
                with open(p, "rb") as f:
                    h.update(f.read())
        return h.hexdigest()
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def home_top(home):
    """Visible top-level names in the home directory (a run may create ~/weekly_budget.csv)."""
    return sorted(n for n in os.listdir(home) if not n.startswith("."))


def snapshot(m, state=STATE):
    base = f"{state}/baseline"
    if os.path.exists(base):
        raise RuntimeError(f"{base} exists: move it aside first, the battery never overwrites a baseline")
    os.makedirs(base)
    for label, src in tracked(m.home).items():
        if os.path.isdir(src):
            shutil.copytree(src, f"{base}/{label}", symlinks=True)
    with open(f"{base}/home-top.txt", "w") as f:
        f.write("\n".join(home_top(m.home)))
    return base


def _move_aside(path, dest_dir, changes, why):
    os.makedirs(dest_dir, exist_ok=True)
    dest = os.path.join(dest_dir, os.path.basename(path))
    n = 1
    while os.path.exists(dest):
        dest = os.path.join(dest_dir, f"{os.path.basename(path)}.{n}")
        n += 1
    shutil.move(path, dest)
    changes.append(f"moved aside ({why}): {path} -> {dest}")


def _restore(src, dst, m, changes, why):
    if os.path.isdir(src):
        shutil.copytree(src, dst, symlinks=True)
    else:
        shutil.copy2(src, dst)
    if hasattr(os, "chown"):
        gid = os.stat(m.home).st_gid
        for root, dirs, files in os.walk(dst):
            for n in dirs + files:
                os.chown(os.path.join(root, n), m.uid, gid)
        os.chown(dst, m.uid, gid)
    changes.append(f"restored ({why}): {dst}")


def reset(m, state=STATE, stamp=None):
    """Make the tracked directories equal the baseline. Returns the list of every change."""
    base = f"{state}/baseline"
    if not os.path.isdir(base):
        raise RuntimeError(f"no baseline at {base}: run with --snapshot-baseline on a machine whose world is as it should be")
    stamp = stamp or time.strftime("%Y%m%d-%H%M%S")
    aside = f"{state}/{stamp}/moved"
    changes = []
    for label, live in tracked(m.home).items():
        b = f"{base}/{label}"
        if not os.path.isdir(b):
            continue
        os.makedirs(live, exist_ok=True)
        live_names, base_names = set(os.listdir(live)), set(os.listdir(b))
        for n in sorted(live_names - base_names):
            _move_aside(f"{live}/{n}", f"{aside}/{label}", changes, "added by a run")
        for n in sorted(base_names & live_names):
            if _digest(f"{live}/{n}") != _digest(f"{b}/{n}"):
                _move_aside(f"{live}/{n}", f"{aside}/{label}", changes, "changed by a run")
                _restore(f"{b}/{n}", f"{live}/{n}", m, changes, "baseline copy")
        for n in sorted(base_names - live_names):
            _restore(f"{b}/{n}", f"{live}/{n}", m, changes, "missing from the world")
    want = set(open(f"{base}/home-top.txt").read().split("\n")) if os.path.exists(f"{base}/home-top.txt") else set()
    for n in home_top(m.home):
        if want and n not in want:
            _move_aside(f"{m.home}/{n}", f"{aside}/home", changes, "added by a run")
    m.act("notes", "reload")
    return changes
