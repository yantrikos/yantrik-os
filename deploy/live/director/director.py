#!/usr/bin/env python3
"""The live instance's Director: gives its Mind long work, one turn at a time, on stream.

Without it the live machine sits on whatever screen it was left on. This is the person at the
keyboard, nothing more: it types a mission into the Lens through `yos act shell send_message`,
the same path a person's typing takes, waits until the Mind has finished its turn, and then says
"keep going" with the mission's next instruction, until the Mind says it is finished, the
mission's turn or time limit is reached, or something needs a person.

It never answers an approval card and never changes the Mind's mode. A card waiting means a
person is needed; the Director logs that, leaves the card up for whoever is watching, and waits.

It does check the work. A mission may name a page (`check_page`), and after every turn the
Director loads it with `check-page`, the same check the Mind is told to run. A page that throws
puts its errors at the head of the next prompt, and a FINISHED is not taken while it throws. On
561 the Mind ticked all eight Starfall steps, and said every feature was in, over a game.js that
did not parse (2 Oct 2026). A tick is the Mind's word; the page loading is the evidence.

It never runs dry. Missions are builds, taken in order; a mission that stopped (a card, a limit,
the screen locked) is picked up again with "keep going", not started over. Each build ends with an
IDEAS.md of improvements, and once every build is finished the Director goes round them in
polish rounds, one improvement a round, the project polished longest ago first. After Starfall the
live machine sat idle for two weeks with every mission finished (2 to 15 Oct 2026). A build whose
code has not changed in STALL_TURNS turns running is stopped and parked for PARK_HOURS, and the
next pick goes to other work until then: on 561 Tumble was resumed for about 120 turns over which
not one of its files changed (18 Oct 2026).

It puts the work on screen. Beside Mind View is a showcase window: a locked-down Chromium
(showcase-browser) showing a frame page with a caption bar (the mission, the turn, the Mind's
latest reply in one line) over the game being built, reloaded after every turn. Between missions
it goes round the finished games a person let into the rotation, in their self-playing demo mode.
The Director only asks (showing.json); a root service copies the game out of ~/Projects and writes
the frame where the Mind's account cannot, and the browser runs as an account of its own that can
see nothing on disk but those copies (showcase_main says why it is three services). On 561 the stream showed only the Mind's
terminal, half the screen was empty, and viewers never saw a game it built (17 Oct 2026). The
showcase is the Director's, never the Mind's: the Mind's own browser still refuses file:// and
new local addresses (check-page says why), and nothing here changes that.

Every mission is a task contract (contracts.py, design/task-contracts-2026-10-17.md): the
Director opens one before the first turn and reports each turn to it, with its evidence: the turn
started, the commands the shell showed, the code that changed (folder hashes), the page check,
a FINISHED taken or refused. A FINISHED it takes makes the contract `delivered`, never
`accepted`: only a person accepts, with director-contracts, as root. The store and the
person's command are out of this account's reach, since the Mind's commands run as it too. And
each report carries the key this run was given (become_director), because what the Director runs
reads the Mind's files, so the Mind's code can run in this unit too.

Builds and polish rounds are interleaved: each finished build is followed by POLISH_PER_BUILD
polish rounds before the next build starts, so the finished games keep getting better while new
ones are made, and the first idea every game gets in polish is its demo mode (DEMO_IDEA).

Runs on the live instance (VM 561) as the desktop's account, from yantrik-live-director.service;
`director.py showcase` is the root publisher, `windows` the window loop, `wayland-grant` the
root step before each browser start (yantrik-live-showcase-grant.service).
Missions are in missions.json beside it. Progress goes to ~/director/log.jsonl, one line a turn.
Stdlib only; the image ships no Python packages for it.
"""
import difflib
import ctypes
import hashlib
import html
import json
import os
import pwd
import re
import shutil
import signal
import socket
import stat
import struct
import subprocess
import sys
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
YOS = "/opt/yantrik/bin/yos"
# yos is Python, and this unit's HOME is the Mind's account's: without -I a .pth the Mind wrote
# under ~/.local would run in every yos the Director starts, inside the Director's unit (security
# review of #857). -I on this unit's own ExecStart is not passed on to a child interpreter.
YOS_RUN = ("/usr/bin/python3", "-I", YOS)
STATE_DIR = os.path.expanduser("~/director")
LOG = os.path.join(STATE_DIR, "log.jsonl")
DONE = os.path.join(STATE_DIR, "done.json")
STARTED = os.path.join(STATE_DIR, "started.json")
POLISHED = os.path.join(STATE_DIR, "polished.json")
# How many polish rounds are still owed before the next build: set after every finished build.
POLISH_DUE = os.path.join(STATE_DIR, "polish-due.json")
# The mission run last, so a build picked up after others knows its conversation is gone.
LAST = os.path.join(STATE_DIR, "last.json")
# Builds parked for having stopped moving, and until when: {build id: seconds since the epoch}.
PARKED = os.path.join(STATE_DIR, "parked.json")
# What the Director asks the showcase to show: a mission and its caption, or nothing (the games go
# round). A request, not the frame: the publisher checks it and builds the frame itself.
SHOWING = os.path.join(STATE_DIR, "showing.json")
CHECK_PAGE = os.path.join(HERE, "check-page")
PROJECTS = os.path.expanduser("~/Projects")

POLL_SECS = 5
# A beat between turns, so someone watching sees the result of one step before the next begins.
BETWEEN_TURNS_SECS = 20
# A turn that has not ended in this long is stuck; the mission stops rather than piling on.
TURN_LIMIT_SECS = 20 * 60
FINISHED = "FINISHED"
# A polish round is one improvement: shorter limits than a build.
POLISH_TURNS = 10
POLISH_HOURS = 1
# Polish rounds after each finished build, before the next build begins.
POLISH_PER_BUILD = 2
# A build whose code has not changed for this many turns running stops and is parked for
# PARK_HOURS: the timer goes on to other work instead of resuming it. On 561 Tumble sat at step
# 4 of 8 for about 120 turns over three runs, not one file changed (18 Oct 2026, #863).
STALL_TURNS = 8
PARK_HOURS = 6

# The showcase. Three services (showcase_main says why): the publisher, as root, copies a game
# out of ~/Projects into SHOWCASE_DIR and writes the frame page there; the browser, as
# SHOWCASE_ACCOUNT, shows it and can see nothing else on disk; the window loop, as the desktop's
# account, puts the window beside Mind View. Neither the Mind's account nor the browser's can
# write SHOWCASE_DIR (root's, 755).
SHOWCASE_DIR = "/opt/yantrik-live/showcase"
GAMES_DIR = os.path.join(SHOWCASE_DIR, "games")
SHOWCASE_TITLE = "Yantrik Live showcase"
SHOWCASE_BROWSER = os.path.join(HERE, "showcase-browser")
SHOWCASE_ACCOUNT = "yantrik-showcase"
BROWSER_UNIT = "yantrik-live-showcase-browser.service"
# The account the Mind's commands run as (agent_run), which owns ~/Projects and ~/director.
MIND_ACCOUNT = "yantrik"
# The finished games a person has let into the rotation, one mission id a line; root's file, so
# the Mind cannot add a game to it or keep one in it. No file, no rotation.
ROTATION = "/etc/yantrik-live/showcase-rotation"
ROTATION_OWNER = 0
# A mission is shown only while the Director's own service is running: the Mind can write
# showing.json, but not start a unit or make its cgroup.
DIRECTOR_CGROUP = "/sys/fs/cgroup/system.slice/yantrik-live-director.service/cgroup.procs"
# Where the grant before the browser (yantrik-live-showcase-grant.service) records which socket it
# was given (wayland_grant).
WAYLAND_RUN = "/run/yantrik-live-showcase"
WAYLAND_SOCKET = "wayland-0"
# A game copied into the showcase is at most this big; a bigger one is not shown at all.
COPY_MAX_FILES = 400
COPY_MAX_BYTES = 48 << 20
COPY_MAX_DEPTH = 6
# Entries looked at, across the whole game, whether copied or not.
COPY_MAX_ENTRIES = 4 * COPY_MAX_FILES
REQUEST_MAX_BYTES = 16 << 10
# The Director's own state files (done.json and the rest) are a few hundred bytes; read_state
# reads no more than this of any of them.
STATE_MAX_BYTES = 1 << 20
# The fastest a running mission's game is copied again (a turn takes longer than this anyway).
LIVE_COPY_SECS = 15
# Copies kept: the one showing, and the one before, which a frame still loading may point at.
GAMES_KEPT = 2
# Between missions, each finished game is shown this long in its demo mode.
CYCLE_SECS = 75
# A mission whose showing has not been refreshed in this long has died with the Director; the
# showcase goes back to cycling the games rather than showing a stopped one for ever.
SHOWING_STALE_SECS = TURN_LIMIT_SECS + 10 * 60
CAPTION_CHARS = 120
# The contracts service (contracts.py serve, yantrik-live-contracts.service). It knows the Director
# by the kernel's account of the caller (this account, in this unit's cgroup) and by this run's key,
# which only this process holds (become_director).
CONTRACTS_SOCKET = "/run/yantrik-contracts/contracts.sock"
# What one event carries at most (contracts.py TEXT_MAX, NAME_MAX), and the commands of one turn
# reported (a turn with more says how many were left out).
CONTRACT_TEXT = 4000
CONTRACT_NAME = 200
CALLS_PER_TURN = 30
# A mission folder, as missions.json may name one: nothing that could climb out of ~/Projects.
FOLDER = re.compile(r"[a-z0-9][a-z0-9-]{0,63}\Z")

# The first improvement every game gets, ahead of its other ideas: something to watch on the
# stream between missions, when nobody is playing it.
DEMO_IDEA = ("a self-playing demo mode: when opened with ?demo=1 it plays itself (attract mode), "
             "visibly, and keeps the score/state changing")

# What every mission is told, filled from its fields in missions.json. One wording for all of them,
# so a lesson learned on one build (agent_run, not the Terminal app; a tick only over a clean
# check) reaches every build.
TERMINAL = ("Do all the work in your own terminal, through the shell's agent_run action: that is how "
            "you make the folder, write each file and look at what you wrote. Do not use the Terminal app.")
BRIEF = (
    "Please build {what} from scratch, in a folder called {folder} inside the Projects folder in your "
    "home folder. {details} Use plain HTML, CSS and JavaScript only, all in that one folder, with no "
    "libraries and no internet. " + TERMINAL + " Start by writing a short plan into a file called "
    "PLAN.md in that folder: a checklist of about {steps} steps, from an empty page to {plan}. Then work "
    "through the plan one step at a time: write or change the files, then check the page by running "
    "`{check}` through agent_run. It loads the page in a headless browser and prints any error the page "
    "throws; no output means it loaded clean. Fix every error it shows. Tick a step off in PLAN.md only "
    "when the code for it is really there and the check is clean: a tick is a promise to the people "
    "watching. People are watching the screen, so keep each step small and visible. When every step is "
    "ticked and the check is clean, write IDEAS.md in the folder: five improvements you would make next, "
    "each an unticked checklist line, the first of them " + DEMO_IDEA + ". Then say FINISHED on its own line.")
KEEP_GOING = (
    "Please keep going on {name}. Read PLAN.md in the {folder} folder through the shell's agent_run "
    "action. If the folder or the plan does not exist yet, make them first, as the brief said. Otherwise "
    "take the first step that is not ticked, do it, and check the page by running `{check}` through "
    "agent_run. Fix any error it prints, and tick the step off in PLAN.md only when its code is really "
    "there and the check is clean. If every step is ticked and the check is clean, write IDEAS.md if it "
    "is not there yet (five improvements, each an unticked checklist line, the first of them " + DEMO_IDEA
    + "), then say FINISHED on its own line.")
# A build picked up after other missions ran in between: on a fresh conversation, which has not
# seen the brief, so it says again what the build is. "Keep going" alone had the Mind build Orrery
# from its name (15 Oct 2026).
RESUME = (
    "Please pick {name} up again where it was left: {what}, in the {folder} folder inside the Projects "
    "folder in your home folder. {details} " + TERMINAL + " Read PLAN.md there, take the first step "
    "that is not ticked, do it, and check the page by running `{check}` through agent_run. Fix any error "
    "it prints, and tick the step off in PLAN.md only when its code is really there and the check is "
    "clean. If every step is ticked and the check is clean, write IDEAS.md if it is not there yet (five "
    "improvements, each an unticked checklist line, the first of them " + DEMO_IDEA + "), then say "
    "FINISHED on its own line.")
FIX_FIRST = (
    "Before anything else on {name}: the page throws an error when it loads. The Director just loaded it "
    "in a headless browser and it said:\n\n{errors}\n\nFix that first, through the shell's agent_run "
    "action, and check it again by running `{check}`. Then read the code and {notes} side by side, and "
    "untick any line whose code is not really there yet: a line is ticked only when it works. Then carry "
    "on with the first line that is not ticked.")
NOTHING_CHANGED = (
    "You said FINISHED on {name}, but nothing in the {folder} folder has changed since this round "
    "started: none of its code changed (ticking a line or copying a file to a backup is not a "
    "change). Reading files is not the step. Do the step now through the shell's "
    "agent_run action, writing the code into the files, check the page by running `{check}`, tick the "
    "line only when its code is really there, and only then say FINISHED on its own line.")
POLISH = (
    "Please make {name} better by one step. It is in the {folder} folder inside the Projects folder in "
    "your home folder. Read IDEAS.md there through the shell's agent_run action. If it is missing, or "
    "every idea in it is ticked, first add three new ideas for improving {name}, each an unticked "
    "checklist line. Take the first idea that is not ticked and do it, keeping the change small and "
    "visible. " + TERMINAL + " Check the page by running `{check}` through agent_run and fix any error "
    "it prints. Tick the idea only when its code is really there and the check is clean. Then say "
    "FINISHED on its own line.")
POLISH_KEEP_GOING = (
    "Please keep going on the {name} idea you are working on, from IDEAS.md in the {folder} folder. "
    "Check the page by running `{check}` through agent_run and fix any error it prints. Tick the idea "
    "only when its code is really there and the check is clean, then say FINISHED on its own line.")


# The publisher sees ~/director read-only and logs to its journal only.
LOG_TO_FILE = [True]


def log(**entry):
    entry["at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    if LOG_TO_FILE[0]:
        # The loops log their own failures through here, so a log file that cannot be written
        # is said in the journal, never raised.
        try:
            os.makedirs(STATE_DIR, exist_ok=True)
            with open(LOG, "a", encoding="utf-8") as f:
                f.write(json.dumps(entry, ensure_ascii=False) + "\n")
        except OSError as why:
            entry["log_file"] = f"not written: {why}"[:200]
    print(json.dumps(entry, ensure_ascii=False), flush=True)


def parse_json(raw):
    """`raw`, bytes of UTF-8 or text, as JSON, or None when it is not JSON: bad UTF-8, bad
    syntax, or nesting deep enough to raise RecursionError (16 KiB of "[" does, #840). Whatever
    the text holds, it is no value rather than an exception; callers cap its size before this."""
    try:
        return json.loads(raw.decode("utf-8") if isinstance(raw, bytes) else raw)
    except Exception:  # ValueError, UnicodeDecodeError, RecursionError, MemoryError: all mean "not JSON"
        return None


def shell_state():
    """The shell's describe state, or None when the desktop does not answer (restarting, locked)."""
    try:
        out = subprocess.run([*YOS_RUN, "describe", "shell"], capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.TimeoutExpired):
        return None
    start, end = out.find("\n{"), out.rfind("\n}")
    if start < 0 or end < 0:
        return None
    state = parse_json(out[start + 1 : end + 2])
    return state if isinstance(state, dict) else None


def last_reply(state):
    turns = state.get("conversation") or []
    return turns[-1] if turns else None


def whole_text(turn):
    """A turn's whole text. `describe` clips each message to 600 characters and marks it
    `clipped`; the Mind ends its reply with FINISHED, which is the part the clip cuts. On 561 every
    Orrery reply ran past the clip and the mission never finished (15 Oct 2026). `read_message`
    returns the rest; if it cannot, the clipped text is all there is."""
    if not turn:
        return ""
    text = turn.get("text") or ""
    if not turn.get("clipped") or turn.get("index") is None:
        return text
    try:
        out = subprocess.run([*YOS_RUN, "act", "shell", "read_message", f"index={turn['index']}"],
                             capture_output=True, text=True, timeout=30).stdout
        start = out.find("\n{")
        whole, _ = json.JSONDecoder().raw_decode(out[start + 1:]) if start >= 0 else ({}, 0)
    except (OSError, subprocess.TimeoutExpired, ValueError, RecursionError):
        return text
    return whole.get("text") or text if isinstance(whole, dict) else text


def said_finished(reply):
    """FINISHED on a line of its own, as every brief asks, bare or in markdown (`**FINISHED**`).
    Not the word inside a sentence: on 561 a polish round was taken as done over "I'm not going to
    write FINISHED on a line for work that isn't there" (16 Oct 2026)."""
    return re.search(rf"(?m)^[^A-Za-z0-9]*{FINISHED}[^A-Za-z0-9]*$", reply) is not None


def mind_idle(state):
    """The Mind has finished its turn: not thinking, and the last word is its own, complete."""
    last = last_reply(state)
    return (not state.get("thinking")) and last is not None and last.get("role") == "assistant" \
        and not last.get("streaming")


def act(action, *args):
    """One shell action, as `yos act shell` takes it. Best effort: (whether it went, what it said)."""
    try:
        done = subprocess.run([*YOS_RUN, "act", "shell", action, *args], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as why:
        return False, str(why)[:300]
    return done.returncode == 0, (done.stdout + done.stderr).strip()[:300]


def show_mind_view():
    """Put Mind View in front, so the stream shows the Mind's work, with the showcase beside it.
    A person can minimise it or open something over it between turns; the person watching should
    not see an idle desktop while the Mind builds. Best effort: Mind View may not exist yet before
    the Mind opens an app.

    Never while a card is waiting: the card is drawn in the shell's Mind panel, and Mind View
    maximised covers the whole shell. On 561 a waiting agent_run card sat behind it and the
    person at the machine looked for it and could not see it (2 Oct 2026)."""
    state = shell_state()
    if state is not None and state.get("pending_approvals"):
        return
    act("focus_window", "title=Mind View")
    place_windows(state)


def snap_action(view):
    """The shell's snap action, from what `describe shell` publishes: (its name, the argument that
    names the window, the argument that says where, the values that one takes), or None when the
    shell publishes none. Looked up rather than assumed, so a shell that renames it or has none
    costs a log line, not a failed mission."""
    for a in (view or {}).get("actions") or []:
        name = a.get("name") or ""
        if not re.search(r"snap(?!shot)", name.lower()):
            continue
        props = (a.get("parameters") or {}).get("properties") or {}
        title = next((k for k in props if "title" in k or k == "window"), None)
        where = next((k for k in props if k != title), None)
        if title and where:
            return name, title, where, [str(v) for v in (props[where] or {}).get("enum") or []]
    return None


def snap_args(snap, title, side):
    """The `yos act shell` arguments that put the window called `title` on `side` (left or right)
    half of the screen, or None when the snap action offers no such place."""
    name, title_arg, where_arg, values = snap
    if values:
        value = next((v for v in values if side in v.lower() and "half" in v.lower()), None) \
            or next((v for v in values if v.lower() == side), None)
    else:
        value = f"{side}-half"
    return None if value is None else [name, f"{title_arg}={title}", f"{where_arg}={value}"]


UNSNAPPED = []


def place_windows(state=None, showcase_front=False):
    """Mind View on the left half of the screen and the showcase on the right, then the showcase
    focused, so focusing Mind View never leaves it covered. Returns whether the windows were
    snapped. Without a snap action they stay where the compositor put them, overlapping: then the
    showcase is brought in front only when `showcase_front` asks (between missions, when it is
    the thing to watch), and during a mission Mind View stays in front."""
    try:
        out = subprocess.run([*YOS_RUN, "describe", "shell", "--json"], capture_output=True, text=True, timeout=30).stdout
        view = parse_json(out)
        snap = snap_action(view) if isinstance(view, dict) else None
    except (OSError, subprocess.TimeoutExpired):
        snap = None
    snapped = False
    if snap is None:
        # Said once a run, not every turn.
        if not UNSNAPPED:
            UNSNAPPED.append(True)
            log(event="windows not snapped", why="the shell publishes no snap action")
    else:
        for title, side in (("Mind View", "left"), (SHOWCASE_TITLE, "right")):
            args = snap_args(snap, title, side)
            snapped = bool(args) and act(*args)[0] and (snapped or side == "left")
    if snapped or showcase_front:
        act("focus_window", f"title={SHOWCASE_TITLE}")
    fill_mind_view(state)
    return snapped


def fill_mind_view(state):
    """Maximise the Mind's terminals inside Mind View. A terminal smaller than Mind View left the
    picture an empty Mind View shows (config/labwc-mind/empty.png, "Apps your companion opens
    appear here, so they stay out of your way.") showing round it, on stream. The picture is right
    for a person's empty Mind View; on 561 the terminal should simply fill it. Mind View is a
    nested labwc on its own display, which `describe shell` names; the Director is its account."""
    display = ((state or {}).get("mind_view") or {}).get("display")
    if not isinstance(display, str) or not re.fullmatch(r"wayland-[0-9]+", display):
        return
    try:
        subprocess.run(["wlrctl", "toplevel", "maximize", "app_id:foot"], capture_output=True, timeout=10,
                       env=dict(os.environ, WAYLAND_DISPLAY=display))
    except (OSError, subprocess.TimeoutExpired):
        pass


def caption_summary(reply):
    """The Mind's reply in one line for the caption: its first sentence, plain text, no longer
    than CAPTION_CHARS. Code blocks, HTML, markdown and paths are taken out; what is left is
    still only text, and the frame page escapes it again."""
    text = html.unescape(re.sub(r"```.*?(```|\Z)", "\n", reply or "", flags=re.S))
    text = re.sub(r"<[^>]*>?", " ", text)
    lines = []
    for line in text.splitlines():
        line = re.sub(r"^\s*(#{1,6}|>|[-*+]|\d+[.)])\s+", "", line)
        line = re.sub(r"^\s*\[[ xX]\]\s*", "", line)
        if line.strip() and not said_finished(line):
            lines.append(line.strip())
    text = " ".join(lines)
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = text.replace("`", " ")
    # A path or an address is any word with a slash in it, whatever punctuation follows it.
    text = re.sub(r"[^\s]*/[^\s]*?(?=[.,;:!?)\]]*(\s|$))", " ", text)
    text = re.sub(r"\*+|~~|(?<!\w)_+|_+(?!\w)", "", text)
    # Controls, and lone surrogates: valid JSON (\ud800) that no encoder takes, so a caption with
    # one failed write_frame on every poll (#853 review).
    text = re.sub(r"[\x00-\x1f\x7f-\x9f\ud800-\udfff]", " ", text)
    text = re.sub(r"\s+([.,;:!?])", r"\1", " ".join(text.split()))
    text = re.sub(r"(\w)\s*:\s*([.!?])", r"\1\2", text)
    first = re.match(r"(.+?[.!?])(\s|$)", text)
    text = first.group(1) if first else text
    if len(text) > CAPTION_CHARS:
        text = text[: CAPTION_CHARS - 1].rsplit(" ", 1)[0].rstrip(" ,;:") + "…"
    return text


class Refused(Exception):
    """A game, or a request, the showcase will not show."""


def open_owned(path, uid, directory=False):
    """`path` opened without following a link anywhere along it, as a file descriptor, only when
    what it names is a directory (or a regular file with one name) owned by `uid`. The publisher
    reads ~/Projects and ~/director as root: a link there, or a hard link to a file the Mind does
    not own, is refused rather than followed, so it reads nothing the Mind could not already."""
    parts = [p for p in path.split("/") if p]
    if not path.startswith("/") or ".." in parts:
        raise Refused("not a plain absolute path")
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for i, part in enumerate(parts):
            last = i == len(parts) - 1
            flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC
            flags |= os.O_DIRECTORY if (directory or not last) else os.O_NONBLOCK
            nxt = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = nxt
        st = os.fstat(fd)
        if st.st_uid != uid:
            raise Refused("not the Mind's account's")
        if not directory and (not stat.S_ISREG(st.st_mode) or st.st_nlink != 1):
            raise Refused("not a plain file")
        return fd
    except OSError as why:
        os.close(fd)
        raise Refused(str(why)) from why
    except Refused:
        os.close(fd)
        raise


def copy_game(folder, uid, version):
    """Copy ~/Projects/<folder> into GAMES_DIR/<folder>-<version>, the only copy of a game the
    browser can see, and return its index.html. Only what a page is made of comes over: regular
    files with one name and directories, owned by the Mind's account, no hidden entries. Links,
    hard links, sockets, pipes and devices are left behind, never followed or opened. A game over
    COPY_MAX_FILES, COPY_MAX_BYTES or COPY_MAX_DEPTH, or with no index.html, is not shown."""
    if not isinstance(folder, str) or not FOLDER.match(folder):
        raise Refused("not a mission folder")
    src = open_owned(os.path.join(PROJECTS, folder), uid, directory=True)
    os.makedirs(GAMES_DIR, mode=0o755, exist_ok=True)
    name = f"{folder}-{version}"
    work = os.path.join(GAMES_DIR, "." + name)
    shutil.rmtree(work, ignore_errors=True)
    os.mkdir(work, 0o755)
    budget = {"files": COPY_MAX_FILES, "seen": COPY_MAX_ENTRIES, "bytes": COPY_MAX_BYTES, "left behind": 0}
    try:
        dst = os.open(work, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            copy_tree(src, dst, uid, 0, budget)
            st = os.stat("index.html", dir_fd=dst, follow_symlinks=False)
            if not stat.S_ISREG(st.st_mode):
                raise Refused("no index.html")
        finally:
            os.close(dst)
        os.rename(work, os.path.join(GAMES_DIR, name))
    except (OSError, Refused) as why:
        shutil.rmtree(work, ignore_errors=True)
        raise Refused(str(why)) from why
    finally:
        os.close(src)
    if budget["left behind"]:
        log(event="showcase copy", folder=folder, left_behind=budget["left behind"])
    return os.path.join(GAMES_DIR, name, "index.html")


def copy_tree(src, dst, uid, depth, budget):
    """One directory of copy_game, by descriptor: every name is looked up in `src` itself, so
    nothing the Mind renames meanwhile can turn the walk outside the game."""
    with os.scandir(src) as entries:
        names = []
        # Read lazily and counted, hidden and left-behind entries too: a folder of a million
        # names is refused before it fills the publisher's memory.
        for entry in entries:
            budget["seen"] -= 1
            if budget["seen"] < 0:
                raise Refused("more than %d entries" % COPY_MAX_ENTRIES)
            if not entry.name.startswith("."):
                names.append(entry.name)
    for name in sorted(names):
        st = os.stat(name, dir_fd=src, follow_symlinks=False)
        if st.st_uid != uid or not (stat.S_ISDIR(st.st_mode) or (stat.S_ISREG(st.st_mode) and st.st_nlink == 1)):
            budget["left behind"] += 1
            continue
        budget["files"] -= 1
        if budget["files"] < 0:
            raise Refused("more than %d files" % COPY_MAX_FILES)
        if stat.S_ISDIR(st.st_mode):
            if depth + 1 >= COPY_MAX_DEPTH:
                raise Refused("deeper than %d folders" % COPY_MAX_DEPTH)
            sub = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=src)
            try:
                if (os.fstat(sub).st_dev, os.fstat(sub).st_ino) != (st.st_dev, st.st_ino):
                    raise Refused("changed while copied")
                os.mkdir(name, 0o755, dir_fd=dst)
                out = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=dst)
                try:
                    copy_tree(sub, out, uid, depth + 1, budget)
                finally:
                    os.close(out)
            finally:
                os.close(sub)
            continue
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=src)
        try:
            now = os.fstat(fd)
            if (now.st_dev, now.st_ino) != (st.st_dev, st.st_ino) or not stat.S_ISREG(now.st_mode) \
                    or now.st_nlink != 1 or now.st_uid != uid:
                raise Refused("changed while copied")
            out = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644, dir_fd=dst)
            try:
                while True:
                    chunk = os.read(fd, 1 << 16)
                    if not chunk:
                        break
                    budget["bytes"] -= len(chunk)
                    if budget["bytes"] < 0:
                        raise Refused("more than %d bytes" % COPY_MAX_BYTES)
                    os.write(out, chunk)
            finally:
                os.close(out)
        finally:
            os.close(fd)


def prune_games(keep):
    """Remove every copy in GAMES_DIR but the ones named in `keep`. Root's directory, written by
    nothing but copy_game, so nothing in it is a link."""
    try:
        names = os.listdir(GAMES_DIR)
    except OSError:
        return
    for name in names:
        if name not in keep:
            shutil.rmtree(os.path.join(GAMES_DIR, name), ignore_errors=True)


FRAME = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline' file:; frame-src file:">
<title>@WINDOW@</title>
<style>
html, body { margin: 0; height: 100%; background: #0c0c14; color: #e8e8f0; font: 16px/1.35 sans-serif; overflow: hidden; }
body { display: flex; flex-direction: column; }
header { padding: 10px 16px 12px; background: #161624; border-bottom: 2px solid #6c5ce7; }
header .top { display: flex; gap: 12px; align-items: baseline; white-space: nowrap; overflow: hidden; }
header .live { background: #e5484d; color: #fff; font-weight: 700; font-size: 12px; padding: 1px 6px; border-radius: 3px; }
header b { font-size: 19px; overflow: hidden; text-overflow: ellipsis; }
header .turn { color: #a8a8c0; }
header p { margin: 6px 0 0; color: #c8c8dc; min-height: 1.35em; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
iframe { flex: 1; width: 100%; border: 0; background: #000; }
.empty { flex: 1; display: flex; align-items: center; justify-content: center; color: #8888a0; font-size: 20px; padding: 24px; text-align: center; }
</style></head>
<body oncontextmenu="return false">
<header><div class="top"><span class="live">LIVE</span><b>@TITLE@</b><span class="turn">@LINE@</span></div><p>@SUMMARY@</p></header>
@GAME@
<script>
var shown = "@VERSION@";
window.showcaseVersion = function (v) { if (v !== shown) location.reload(); };
setInterval(function () {
  var s = document.createElement("script");
  s.src = "version.js?" + Date.now();
  s.onload = s.onerror = function () { s.remove(); };
  document.head.appendChild(s);
}, 3000);
</script>
</body></html>
"""


def frame_page(title, line, summary, url, empty, version):
    """The frame page's HTML. Every word in it that came from a mission or the Mind is escaped,
    quotes too, so a reply with markup in it is shown as text and can never become markup."""
    if url:
        # allow-same-origin is kept so a game's high scores (localStorage) work: without it the
        # first storage call throws and the game stops on stream. It is safe only while the game
        # and this frame are different origins, which every file:// page is in Chromium, each an
        # opaque origin of its own. Never add --allow-file-access-from-files to showcase-browser,
        # and never serve the frame and the game from one http origin: with scripts and the same
        # origin together, the game could reach parent.document and lift its own sandbox.
        game = ('<iframe src="%s" sandbox="allow-scripts allow-same-origin" title="%s"></iframe>'
                % (html.escape(url, quote=True), html.escape(title, quote=True)))
    else:
        game = '<div class="empty">%s</div>' % html.escape(empty, quote=True)
    page = FRAME
    # The version last: it is hex, and nothing filled in before it can be mistaken for its marker.
    for mark, value in (("@WINDOW@", html.escape(SHOWCASE_TITLE)), ("@TITLE@", html.escape(title, quote=True)),
                        ("@LINE@", html.escape(line, quote=True)), ("@SUMMARY@", html.escape(summary, quote=True))):
        page = page.replace(mark, value, 1)
    return page.replace("@GAME@", game, 1).replace("@VERSION@", version, 1)


def write_atomically(path, text):
    with open(path + ".new", "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(path + ".new", path)


def write_frame(title, line, summary, folder, uid, demo=True, empty="Nothing to show yet."):
    """Copy `folder`'s game into the showcase and rewrite the frame page for it, then its version,
    which the page polls: a new version is the page reloading itself, game and all. The publisher
    only. Returns the game's URL, or None when it was not shown (and `empty` was)."""
    version = hashlib.sha256(f"{title}|{line}|{summary}|{folder}|{time.time()}".encode()).hexdigest()[:16]
    url = None
    if folder is not None:
        try:
            url = "file://" + urllib.parse.quote(copy_game(folder, uid, version)) + ("?demo=1" if demo else "")
        except Refused as why:
            log(event="game not shown", folder=folder if isinstance(folder, str) else None, why=str(why)[:200])
    write_atomically(os.path.join(SHOWCASE_DIR, "frame.html"), frame_page(title, line, summary, url, empty, version))
    write_atomically(os.path.join(SHOWCASE_DIR, "version.js"), 'showcaseVersion("%s");\n' % version)
    kept = sorted((n for n in os.listdir(GAMES_DIR) if not n.startswith(".")),
                  key=lambda n: os.stat(os.path.join(GAMES_DIR, n)).st_mtime)[-GAMES_KEPT:] \
        if os.path.isdir(GAMES_DIR) else []
    prune_games(set(kept))
    return url


def show_mission(m, turn=0, reply="", clean=None):
    """Ask the showcase for the mission's game, with the caption for this turn: the Director's
    side, as the desktop's account. Only a request; the publisher checks it and builds the frame.
    Never stops a mission: a showcase that cannot be asked costs the stream, not the work."""
    try:
        write_state(SHOWING, {"mission": m["id"], "at": time.time(), "turn": turn, "clean": clean,
                              "summary": caption_summary(reply)})
    except OSError as why:
        log(mission=m["id"], event="showcase not asked", why=str(why)[:200])


# This run's key from the contracts service (become_director), sent with every report. Only in
# this process's memory: never in a file, the environment or an argument, so nothing this
# process starts is given it.
DIRECTOR_KEY = []
PR_SET_DUMPABLE = 4


def become_director():
    """Make this process one no other process of its account can look into, then ask the
    contracts service for this run's key. Called before the Director starts any process: the
    service gives the key only to the one process in the unit. The unit is not enough to be the
    Director, since what it starts (yos, check-page, chromium) reads files the Mind wrote and so
    can run the Mind's code in it, as this account (security review of #857). Not dumpable, this
    process's /proc entries are root's: its memory, environment and open files cannot be read by
    the account's other processes, the ones in this unit included, whatever ptrace_scope is.
    Returns why not, or None."""
    try:
        if ctypes.CDLL(None, use_errno=True).prctl(PR_SET_DUMPABLE, 0, 0, 0, 0) != 0:
            return "this process could not be made non-dumpable"
    except (OSError, AttributeError) as why:
        return f"this process could not be made non-dumpable: {why}"
    reply = contract_call({"op": "register"})
    if not (reply or {}).get("ok") or not isinstance(reply.get("key"), str):
        return str((reply or {}).get("error") or "the contracts service did not answer")[:200]
    DIRECTOR_KEY[:] = [reply["key"]]
    return None


def contract_call(request):
    """One request to the contracts service, its reply as a dict, or None when it did not answer.
    Tried twice: the service restarts in a second. Every request but `register` carries the key."""
    if request.get("op") != "register":
        request = dict(request, key=DIRECTOR_KEY[0] if DIRECTOR_KEY else "")
    for attempt in range(2):
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                s.settimeout(10)
                s.connect(CONTRACTS_SOCKET)
                # As UTF-8, not escaped: an event at its limits then fits the service's request cap.
                s.sendall(json.dumps(request, ensure_ascii=False).encode())
                s.shutdown(socket.SHUT_WR)
                raw = b""
                while len(raw) <= REQUEST_MAX_BYTES:
                    chunk = s.recv(1 << 16)
                    if not chunk:
                        break
                    raw += chunk
            reply = parse_json(raw)
            return reply if isinstance(reply, dict) else None
        except OSError:
            if attempt == 0:
                time.sleep(1)
    return None


class Contract:
    """A mission's contract as the Director holds it: its id and revision, and what it reports.
    The Director only reports; nothing it can send accepts a contract (contracts.py EVENTS)."""

    def __init__(self, cid, revision):
        self.id, self.revision, self.ended = cid, revision, False

    def event(self, kind, **fields):
        reply = contract_call({"op": "event", "contract": self.id, "revision": self.revision, "kind": kind, **fields})
        if not (reply or {}).get("ok"):
            # A lost event costs the record, not the mission: said in the log, and the next goes on.
            log(contract=self.id, event="contract event not recorded", kind=kind,
                why=str((reply or {}).get("error") or "the contracts service did not answer")[:200])
        # Ended only once the service has it: otherwise run_mission's finally says the mission
        # stopped, so the contract is never left running behind a Director that has gone.
        if kind in ("finished_said", "stopped", "failed") and (reply or {}).get("ok"):
            self.ended = True
        return reply


def open_contract(m, resuming):
    """The mission's contract, opened or picked up again: the brief, whole, is its intent. None
    when the service did not take it; then the mission does not start (no contract, no mission)."""
    reply = contract_call({"op": "open", "mission": m["id"], "title": m["title"], "brief": prompt(m["brief"], m),
                           "folder": m["fields"]["folder"], "page": m.get("check_page") or "", "resume": resuming})
    if not (reply or {}).get("ok"):
        log(mission=m["id"], event="not started", why="no contract: " + str((reply or {}).get("error")
                                                                             or "the contracts service did not answer")[:200])
        return None
    return Contract(reply["contract"], reply["revision"])


def print_hash(p):
    """A folder_print as one sha256: the evidence the contract keeps of what the code was."""
    return hashlib.sha256(json.dumps(p, default=str).encode()).hexdigest()


def changed_files(a, b):
    """The names whose content differs between two folder prints, as many and as long as an event
    takes (contracts.py FILES_MAX, NAME_MAX): a longer name would have the whole event refused."""
    try:
        return [name[:CONTRACT_NAME] for name in sorted({name for name, _ in set(a) ^ set(b)})[:50]]
    except (TypeError, ValueError):
        return []


def turn_calls(state):
    """The Mind's tool calls in the turn just ended, as the shell read them (`calls` in describe:
    what the person's panel shows), one line each: what was asked to run, not that it ran well."""
    turns = (state or {}).get("conversation") or []
    turns = [t for t in turns if isinstance(t, dict)]
    since = max((i for i, t in enumerate(turns) if t.get("role") == "user"), default=-1)
    lines = []
    for t in turns[since + 1:]:
        for c in t.get("calls") or []:
            if not isinstance(c, dict):
                continue
            what = c.get("summary") if isinstance(c.get("summary"), str) else json.dumps(c.get("arguments"))[:300]
            lines.append(f"{str(c.get('name'))[:60]}: {' '.join(str(what).split())}"[:CONTRACT_NAME * 2])
    return lines


def new_chat():
    """Start the Mind on a fresh conversation, so a mission begins from its brief and not from
    the day's history. On 561 the Mind answered nine "keep going" turns in a second each with
    "every command needs your approval", calling nothing, while a session rule allowed every
    command: earlier missions' lapsed cards were still in the conversation (16 Oct 2026).
    Best effort: a desktop that will not start one leaves the old conversation, which only costs
    what it cost before."""
    done = subprocess.run([*YOS_RUN, "act", "shell", "new_chat"], capture_output=True, text=True, timeout=60)
    return done.returncode == 0, (done.stdout + done.stderr).strip()[:300]


def same_answer(a, b):
    """Whether two replies are the same answer: the Mind saying again what it just said, give or
    take a word ("I can't make progress" / "I still can't make progress"). Only the opening is
    compared, where the answer is; a long reply that starts alike and goes on differently is a
    different turn."""
    a, b = " ".join(a.split())[:400], " ".join(b.split())[:400]
    return bool(a) and bool(b) and difflib.SequenceMatcher(None, a, b).ratio() >= 0.9


def send(text):
    done = subprocess.run([*YOS_RUN, "act", "shell", "send_message", f"text={text}"],
                          capture_output=True, text=True, timeout=60)
    return done.returncode == 0, (done.stdout + done.stderr).strip()[:300]


def wait_for_turn(sent_at):
    """Wait until the Mind answers what was just sent. Returns (state, why it stopped waiting,
    whether a card rose during the turn)."""
    seen_busy = False
    card_seen = False
    while True:
        time.sleep(POLL_SECS)
        state = shell_state()
        if state is None:
            if time.time() - sent_at > TURN_LIMIT_SECS:
                return None, "desktop not answering", card_seen
            continue
        if state.get("locked"):
            return state, "locked", card_seen
        if state.get("pending_approvals"):
            # A card is the Mind waiting for a person, not the turn ending: keep waiting (up to
            # the turn limit) so a person who answers it finds the mission still going. On 561
            # the Director stopped the moment the first card rose, before Pranab could answer.
            if not card_seen:
                card_seen = True
                log(event="card waiting", actions=[a.get("action") for a in state["pending_approvals"]])
            if time.time() - sent_at > TURN_LIMIT_SECS:
                return state, "card unanswered", card_seen
            continue
        if state.get("thinking"):
            seen_busy = True
        if (seen_busy or time.time() - sent_at > 30) and mind_idle(state):
            return state, "answered", card_seen
        if time.time() - sent_at > TURN_LIMIT_SECS:
            return state, "turn too long", card_seen


def check_page(m):
    """Load the mission's page and return (clean, errors). clean is None when there is no page to
    check or it could not be checked (no chromium, a hang), which neither holds up nor passes."""
    page = m.get("check_page")
    if not page:
        return None, ""
    try:
        done = subprocess.run([CHECK_PAGE, os.path.expanduser(page)], capture_output=True, text=True, timeout=90)
    except (OSError, subprocess.TimeoutExpired) as why:
        return None, str(why)[:200]
    said = done.stdout.strip()[:1500]
    if done.returncode == 0:
        return True, ""
    if done.returncode == 1:
        return False, said
    return None, said


def read_state(path, empty):
    """The state file at `path` as the type of `empty`, or `empty` when it is missing, not a plain
    file, over STATE_MAX_BYTES, not JSON or not that type. The Mind's account can write ~/director,
    so this reads hostile bytes too (SHOWING in windows_main): never past the cap, never blocking
    on a FIFO, never raising."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC)
    except OSError:
        return empty
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            return empty
        raw = os.read(fd, STATE_MAX_BYTES + 1)
    except OSError:
        return empty
    finally:
        os.close(fd)
    value = parse_json(raw) if len(raw) <= STATE_MAX_BYTES else None
    try:
        return type(empty)(value) if value is not None else empty
    except (ValueError, TypeError, OverflowError):
        return empty


def write_state(path, value):
    os.makedirs(STATE_DIR, exist_ok=True)
    with open(path + ".new", "w", encoding="utf-8") as f:
        json.dump(sorted(value) if isinstance(value, set) else value, f)
    os.replace(path + ".new", path)


def finished_missions():
    return read_state(DONE, set())


def mark_finished(mission_id):
    write_state(DONE, finished_missions() | {mission_id})
    # A finished build earns the finished games their polish rounds before the next build.
    write_state(POLISH_DUE, POLISH_PER_BUILD)


def parks(now=None):
    """{mission id: until} for the builds parked now, from PARKED. ~/director is the Mind's
    account's to write, so the file is hostile input, read only through read_state: a value that
    is not a number is dropped, every `until` is clamped to at most now + PARK_HOURS, and one
    already past is gone (next_mission writes the file back so). Only known build ids count,
    because only next_mission's unfinished builds are looked up in it. A hostile writer can at
    most park a build for PARK_HOURS from its last write, or unpark it early: the same trust as
    DONE, where it can mark any build finished today."""
    now = time.time() if now is None else now
    latest = now + PARK_HOURS * 3600
    kept = {}
    for mission_id, until in read_state(PARKED, {}).items():
        # Not float(): an integer of a thousand digits does not fit one, and compares as it is.
        if isinstance(mission_id, str) and isinstance(until, (int, float)) and not isinstance(until, bool) \
                and until > now:
            kept[mission_id] = min(until, latest)
    return kept


def park(mission_id):
    """Park the build for PARK_HOURS from now; returns until when."""
    until = time.time() + PARK_HOURS * 3600
    write_state(PARKED, {**parks(), mission_id: until})
    return until


def unpark(mission_id):
    """Lift the build's park, if it has one."""
    parked = parks()
    if parked.pop(mission_id, None) is not None:
        write_state(PARKED, parked)


def build(spec):
    """A build mission from its entry in missions.json: the shared wording, filled in."""
    folder = spec["folder"]
    page = f"~/Projects/{folder}/index.html"
    fields = dict(name=spec["name"], what=spec["what"], details=spec["details"], plan=spec["plan"],
                  steps=spec.get("steps", 8), folder=folder, check=f"{CHECK_PAGE} {page}", notes="PLAN.md")
    return dict(id=spec["id"], title=spec["title"], fields=fields, check_page=page, brief=BRIEF,
                keep_going=KEEP_GOING, max_turns=spec.get("max_turns", 40), max_hours=spec.get("max_hours", 3))


def polish(b):
    """A polish round on a finished build: one improvement from its IDEAS.md."""
    fields = dict(b["fields"], notes="IDEAS.md")
    return dict(b, id="polish:" + b["id"], title="Polishing " + b["fields"]["name"], fields=fields,
                brief=POLISH, keep_going=POLISH_KEEP_GOING, max_turns=POLISH_TURNS, max_hours=POLISH_HOURS)


def is_code(name):
    """Whether a file is part of what the page is made of, rather than notes or a backup."""
    lower = name.lower()
    if lower.endswith((".md", "~", ".orig", ".tmp", ".swp")) or ".bak" in lower:
        return False
    return True


# The most of one file the folder print reads; past it the file counts by its first bytes and size.
PRINT_FILE_MAX = 8 << 20


def file_digest(path):
    """sha256 of the plain file at `path` (its first PRINT_FILE_MAX bytes and its size), or None
    for anything else. The folder is the Mind's: a FIFO named game.js would block the Director
    and a link to /dev/zero would read without end, so links are not followed, nothing blocks,
    and the read is capped (pre-review grep of #857)."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    except OSError:
        return None
    try:
        st = os.fstat(fd)
        if not stat.S_ISREG(st.st_mode):
            return None
        h, left = hashlib.sha256(), PRINT_FILE_MAX
        while left > 0:
            chunk = os.read(fd, min(left, 1 << 20))
            if not chunk:
                break
            h.update(chunk)
            left -= len(chunk)
        if st.st_size > PRINT_FILE_MAX:
            h.update(b"\0size %d" % st.st_size)
        return h.hexdigest()
    except OSError:
        return None
    finally:
        os.close(fd)


def folder_print(m):
    """What the mission's code holds, by name and content: two prints differ when a file the
    page is made of was really changed. Notes (`.md`), backups (`.bak`, `~`) and hidden entries
    are not counted: on 561 a Labyrinth round was taken as done when the Mind ticked IDEAS.md
    and copied maze.js to maze.js.bak, while maze.js itself was byte for byte what it had been
    (16 Oct 2026). Content, not modification time, so rewriting a file unchanged is not a
    change either. A missing folder prints empty."""
    root = os.path.join(PROJECTS, m["fields"]["folder"])
    seen = []
    for top, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if not x.startswith("."))
        for name in sorted(files):
            if name.startswith(".") or not is_code(name):
                continue
            digest = file_digest(os.path.join(top, name))
            if digest is None:
                continue
            seen.append((os.path.relpath(os.path.join(top, name), root), digest))
    return tuple(seen)


def prompt(template, m, errors=""):
    return template.format(errors=errors, **m["fields"])


def next_mission(builds):
    """The first unfinished build, unless polish rounds are owed: after each finished build, the
    next POLISH_PER_BUILD missions are polish rounds, so builds and polish take turns rather than
    every build first and then polish only. Once all builds are finished, polish rounds only. A
    polish round goes to the finished build polished longest ago (never polished first, then in
    the missions' order).

    A parked build (parks) is passed over until its park ends, and then resumed where it was.
    With every unfinished build parked, a polish round runs instead; with no finished build to
    polish either, nothing does, and the timer tries again after its rest."""
    done = finished_missions()
    todo = [b for b in builds if b["id"] not in done]
    finished = [b for b in builds if b["id"] in done]
    parked = parks()
    # Written back clamped and without what was dropped: clamped only as read, a planted
    # `until` of 1e18 would be now + PARK_HOURS again at every pick, a park for ever.
    if parked != read_state(PARKED, {}):
        write_state(PARKED, parked)
    ready = [b for b in todo if b["id"] not in parked]
    due = read_state(POLISH_DUE, 0)
    if ready and (due <= 0 or not finished):
        return ready[0]
    if not finished:
        log(event="idle", why="every build is parked" if todo else "there are no missions")
        return None
    if todo and not ready:
        log(event="every build is parked", then="a polish round")
    if ready:
        write_state(POLISH_DUE, due - 1)
    # Each build's last polish round, by number: rounds are counted, not timed, so two rounds in
    # the same second still go round in turn.
    polished = read_state(POLISHED, {})
    b = min(finished, key=lambda b: (polished.get(b["id"], 0), builds.index(b)))
    polished[b["id"]] = max(polished.values(), default=0) + 1
    write_state(POLISHED, polished)
    return polish(b)


def ensure_demo_idea(m):
    """Put DEMO_IDEA at the head of the game's IDEAS.md, unticked, unless the file already has a
    line about ?demo=1 (ticked or not), so the polish round takes it before the game's other
    ideas. The Director's one write into a mission's folder, and only to its notes: a note is not
    code, so it never counts as the Mind having changed anything (folder_print)."""
    folder = m["fields"]["folder"]
    if not FOLDER.match(folder) or not os.path.isdir(os.path.join(PROJECTS, folder)):
        return False
    path = os.path.join(PROJECTS, folder, "IDEAS.md")
    if os.path.islink(path):
        return False
    # Not open(): a FIFO there would block the Director (pre-review grep of #857).
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    except FileNotFoundError:
        fd = None
    except OSError:
        return False
    try:
        if fd is None:
            lines = []
        elif not stat.S_ISREG(os.fstat(fd).st_mode):
            return False
        else:
            raw = os.read(fd, STATE_MAX_BYTES + 1)
            if len(raw) > STATE_MAX_BYTES:
                return False
            lines = raw.decode("utf-8").splitlines()
    except (OSError, ValueError):
        return False
    finally:
        if fd is not None:
            os.close(fd)
    if any("demo=1" in line for line in lines):
        return False
    at = next((i for i, line in enumerate(lines) if re.match(r"\s*[-*+]\s+\[[ xX]\]", line)), len(lines))
    lines.insert(at, "- [ ] " + DEMO_IDEA[0].upper() + DEMO_IDEA[1:])
    write_atomically(path, "\n".join(lines) + "\n")
    return True


def run_mission(m):
    started = time.time()
    if m["id"].startswith("polish:"):
        try:
            if ensure_demo_idea(m):
                log(mission=m["id"], event="demo idea added", to="IDEAS.md")
        except OSError as why:
            log(mission=m["id"], event="demo idea not added", why=str(why)[:200])
    # A build stopped part way (a card, a limit, the screen locked) is picked up where it is, not
    # started over: the brief would have the Mind write a fresh plan over its own ticks.
    begun = read_state(STARTED, set())
    # A mission that runs is not parked, whoever chose it: next_mission once the park was over,
    # or a person naming it.
    unpark(m["id"])
    # Only once the plan exists: a start that never reached the Mind (the model unreachable, a
    # card) left nothing to keep going on, and "keep going" carries none of the brief. On 561 the
    # Mind built Orrery from its name alone and said it had never seen the brief (15 Oct 2026).
    planned = os.path.exists(os.path.join(PROJECTS, m["fields"]["folder"], "PLAN.md"))
    resuming = m["id"] in begun and planned and not m["id"].startswith("polish:")
    # Picked up after other missions ran: their conversation, not this one's, is on screen, so it
    # starts afresh and is told again what the build is.
    elsewhere = resuming and read_state(LAST, "") != m["id"]
    contract = open_contract(m, resuming)
    if contract is None:
        return
    try:
        run_turns(m, contract, started, resuming, elsewhere, begun)
    finally:
        # However the turns ended without saying so (an error, the service stopped: SIGTERM is
        # SystemExit, main), the contract says the mission stopped and is not left running.
        if not contract.ended:
            contract.event("stopped", turn=0, why="the Director stopped before the mission ended")


def stop(m, contract, why, turns):
    log(mission=m["id"], event="stopped", why=why, turns=turns, contract=contract.id)
    contract.event("stopped", turn=turns, why=why)


def park_stalled(m, contract, turn, clean):
    """Stop a build whose code has not changed in STALL_TURNS turns, park it, and say so on the
    stream. The contract gets its usual `stopped`; the park is the Director's own record."""
    until = time.time() + PARK_HOURS * 3600
    log(mission=m["id"], event="parked", until=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(until)),
        stalled_turns=STALL_TURNS)
    show_mission(m, turn, f"Parking {m['fields']['name']}: {STALL_TURNS} turns without a change to its files; "
                 "back to it later.", clean)
    stop(m, contract, f"no progress in {STALL_TURNS} turns", turn)
    try:
        park(m["id"])
    except OSError as why:
        # Unrecorded, the build is only resumed as before the rule: said, not raised.
        log(mission=m["id"], event="park not recorded", why=str(why)[:200])
    # Long enough for the publisher (at most one copy every LIVE_COPY_SECS) to put the line on
    # screen before the mission's end hands the showcase back to the rotation.
    time.sleep(LIVE_COPY_SECS + BETWEEN_TURNS_SECS)


def run_turns(m, contract, started, resuming, elsewhere, begun):
    write_state(STARTED, begun | {m["id"]})
    write_state(LAST, m["id"])
    log(mission=m["id"], event="resuming" if resuming else "starting", title=m["title"], contract=contract.id,
        revision=contract.revision)
    text = prompt(RESUME if elsewhere else m["keep_going"] if resuming else m["brief"], m)
    show_mission(m)
    if not resuming or elsewhere:
        ok, said = new_chat()
        if not ok:
            log(mission=m["id"], event="fresh chat refused", said=said)
    cards = 0
    previous = ""
    # A FINISHED counts only over a folder that changed. On 561 the Mind read Beatgrid's files
    # eight times, wrote nothing, and said FINISHED over a description of code it had not
    # written; the page, untouched, checked clean (16 Oct 2026).
    before = folder_print(m)
    last_print = before
    # Turns running in which the code did not change: a build stalled for STALL_TURNS is parked.
    stalled = 0
    for turn in range(1, m["max_turns"] + 1):
        if time.time() - started > m["max_hours"] * 3600:
            return stop(m, contract, "time limit", turn - 1)
        # Someone (or the Mind on its own) may be mid-turn: let it finish before typing over it.
        for _ in range(TURN_LIMIT_SECS // POLL_SECS):
            state = shell_state()
            if state is not None and not state.get("thinking"):
                break
            time.sleep(POLL_SECS)
        show_mind_view()
        contract.event("turn_started", turn=turn, prompt=text[:CONTRACT_TEXT])
        ok, said = send(text)
        if not ok:
            log(mission=m["id"], event="send failed", turn=turn, said=said)
            return stop(m, contract, "the message could not be sent", turn)
        state, why, card = wait_for_turn(time.time())
        reply = whole_text(last_reply(state)) if state else ""
        clean, errors = check_page(m)
        page = {True: "loads clean", False: "throws", None: "not checked"}[clean]
        log(mission=m["id"], event="turn", turn=turn, outcome=why, reply=reply[:400], page=page, errors=errors or None)
        # The turn's record on the contract: what the Mind ran, what changed, what the page did.
        calls = turn_calls(state)
        for line in calls[:CALLS_PER_TURN]:
            contract.event("command_run", turn=turn, command=line)
        if len(calls) > CALLS_PER_TURN:
            contract.event("command_run", turn=turn, command=f"and {len(calls) - CALLS_PER_TURN} more")
        now = folder_print(m)
        stalled = stalled + 1 if now == last_print else 0
        if now != last_print:
            contract.event("file_changed", turn=turn, before=print_hash(last_print), after=print_hash(now),
                           files=changed_files(last_print, now))
            last_print = now
        contract.event("page_check", turn=turn, result=page, output=errors[:CONTRACT_TEXT])
        # The game reloaded in the showcase after every turn, its page checked first.
        show_mission(m, turn, reply, clean)
        if why != "answered":
            return stop(m, contract, why, turn)
        # A card the Mind gave up waiting on ends its turn as if answered, and "keep going" only
        # raises the next one. Two turns running with a card is a person needed: stop, and the
        # timer tries again after its rest. On 561 the first agent_run of a new Mind session asks,
        # and each card lapsed in 110 s while the Director kept typing (15 Oct 2026).
        cards = cards + 1 if card else 0
        if cards >= 2:
            return stop(m, contract, "cards are waiting for a person", turn)
        unchanged = said_finished(reply) and now == before
        if unchanged:
            log(mission=m["id"], event="finished refused", turn=turn, why="nothing in the folder changed")
            contract.event("finished_refused", turn=turn, why="nothing in the folder changed",
                           mind_said=reply[-CONTRACT_TEXT:])
        elif said_finished(reply):
            if clean is False:
                # The Mind's word against the page's: the page wins, and the mission goes on.
                log(mission=m["id"], event="finished refused", turn=turn, why="the page throws")
                contract.event("finished_refused", turn=turn, why="the page throws", mind_said=reply[-CONTRACT_TEXT:])
            else:
                if not m["id"].startswith("polish:"):
                    mark_finished(m["id"])
                # Delivered, not accepted: the checks are the Director's heuristic, and only a
                # person's decision (director-contracts accept) accepts the work.
                contract.event("finished_said", turn=turn, mind_said=reply[-CONTRACT_TEXT:], folder=print_hash(now),
                               page=page)
                log(mission=m["id"], event="finished", turns=turn, page_checked=clean is True,
                    minutes=round((time.time() - started) / 60), contract=contract.id, state="delivered")
                return
        # Polish rounds keep their own short limits (POLISH_TURNS); only builds are parked.
        if stalled >= STALL_TURNS and not m["id"].startswith("polish:"):
            return park_stalled(m, contract, turn, clean)
        # The same answer twice is a Mind that is not working the mission, whatever the reason;
        # typing "keep going" at it again only fills the stream with the same words.
        if same_answer(reply, previous):
            return stop(m, contract, "the same answer twice", turn)
        previous = reply
        time.sleep(BETWEEN_TURNS_SECS)
        if clean is False:
            text = prompt(FIX_FIRST, m, errors)
        elif unchanged:
            text = prompt(NOTHING_CHANGED, m)
        else:
            text = prompt(m["keep_going"], m)
    stop(m, contract, "turn limit", m["max_turns"])


def rotation():
    """The mission ids a person has let into the rotation (ROTATION), in the file's order. Read
    only when root owns the file and nobody else may write it; otherwise none."""
    try:
        with open(ROTATION, encoding="utf-8") as f:
            st = os.fstat(f.fileno())
            if st.st_uid != ROTATION_OWNER or st.st_mode & 0o022:
                return []
            text = f.read(REQUEST_MAX_BYTES)
    except (OSError, ValueError):
        return []
    ids = []
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if line and line not in ids:
            ids.append(line)
    return ids


def director_running():
    """Whether yantrik-live-director.service has a process: only systemd makes that cgroup."""
    try:
        with open(DIRECTOR_CGROUP, encoding="ascii") as f:
            return bool(f.read(64).strip())
    except (OSError, ValueError):
        return False


def read_request(uid):
    """The Director's request (show_mission), checked: a mission of missions.json, its turn and
    page state, and the one line of caption. Anything else in the file is ignored."""
    try:
        fd = open_owned(SHOWING, uid)
    except Refused:
        return {}
    try:
        raw = os.read(fd, REQUEST_MAX_BYTES + 1)
    except OSError:
        return {}
    finally:
        os.close(fd)
    req = parse_json(raw) if len(raw) <= REQUEST_MAX_BYTES else None
    return req if isinstance(req, dict) else {}


def mission_for(builds, mission_id):
    """The mission `mission_id` names, a build or a polish round of one, or None."""
    if not isinstance(mission_id, str):
        return None
    for b in builds:
        if mission_id == b["id"]:
            return b
        if mission_id == "polish:" + b["id"]:
            return polish(b)
    return None


def live_request(builds, uid):
    """(the mission to show, its turn line, its caption, its time) while the Director runs one, or
    None. Everything shown comes from missions.json or is made here, but the caption, which is
    the Mind's reply in one line, as the stream already shows it, made plain again and escaped."""
    if not director_running():
        return None
    req = read_request(uid)
    m = mission_for(builds, req.get("mission"))
    try:
        fresh = time.time() - float(req.get("at") or 0) < SHOWING_STALE_SECS
    except (TypeError, ValueError):
        fresh = False
    if m is None or not fresh:
        return None
    turn = req.get("turn")
    turn = turn if isinstance(turn, int) and not isinstance(turn, bool) and 0 <= turn <= 10 ** 4 else 0
    line = f"turn {turn}" if turn else "starting"
    if req.get("clean") is False:
        line += " · the page throws an error, being fixed"
    summary = req.get("summary")
    summary = caption_summary(summary[:4000]) if isinstance(summary, str) else ""
    return m, line, summary, req.get("at")


def cycle_step(builds, index, uid):
    """Show the next game of the rotation in the showcase, in its demo mode, between missions.
    Returns the index to show next time. Only builds a person put in ROTATION are shown, in
    missions.json's order, and one that cannot be copied is passed over."""
    allowed = set(rotation())
    games = [b for b in builds if b["id"] in allowed]
    for step in range(len(games)):
        b = games[(index + step) % len(games)]
        if write_frame(b["title"], "between missions · demo", "Built earlier by the Mind; its next mission starts soon.",
                       b["fields"]["folder"], uid):
            return (index + step + 1) % len(games)
    write_frame("Yantrik Live", "between missions", "The Mind's next mission starts soon.", None, uid,
                empty="No game is in the rotation yet: the Mind is building the next one.")
    return 0


def showcase_main(builds):
    """yantrik-live-showcase.service, as root: what the showcase shows. While the Director runs
    a mission, that mission's game as it is now, re-copied every turn; otherwise the rotation,
    one game every CYCLE_SECS.

    Split in three because no one account may do it all. The game is the Mind's code, so the
    browser that runs it (yantrik-live-showcase-browser.service) is an account of its own that
    sees only SHOWCASE_DIR, and cannot write that. The Mind's account cannot write it either, so
    the copy and the frame are root's, here, in a unit that can read ~/Projects and ~/director and
    write SHOWCASE_DIR, nothing else. Placing the window takes `yos` as the desktop's account,
    which is windows_main."""
    LOG_TO_FILE[0] = False
    uid = pwd.getpwnam(MIND_ACCOUNT).pw_uid
    index, shown_at, last = 0, 0.0, None
    while True:
        # Each step is on its own: a request that fails in a way not foreseen here, or a step that
        # does, is logged and the next poll starts afresh. On #840 a RecursionError escaped this
        # loop, and Restart=always only brought it back to the same file, the showcase frozen.
        try:
            live = live_request(builds, uid)
            if live is not None:
                m, line, summary, at = live
                key = (m["id"], line, summary, at)
                # A new turn is a new copy, but not more often than LIVE_COPY_SECS: the Mind can
                # rewrite the request as often as it likes, and each copy is up to COPY_MAX_BYTES.
                if key != last and time.time() - shown_at >= LIVE_COPY_SECS:
                    write_frame(m["title"], line, summary, m["fields"]["folder"], uid,
                                empty=f"{m['fields']['name']} has no page yet: the Mind is starting on it.")
                    shown_at, last = time.time(), key
            elif last is not None or time.time() - shown_at >= CYCLE_SECS:
                index = cycle_step(builds, index, uid)
                shown_at, last = time.time(), None
        except OSError as why:
            log(event="showcase not written", why=str(why)[:200])
        except Exception as why:  # noqa: BLE001 - one bad step never stops the showcase
            log(event="showcase step failed", why=f"{type(why).__name__}: {why}"[:200])
        time.sleep(POLL_SECS)


def browser_started():
    """The showcase browser's current start (its unit's invocation id), or None when it is not
    running. The window loop places the window again each time this changes."""
    try:
        done = subprocess.run(["systemctl", "show", "-p", "ActiveState,InvocationID", "--value", BROWSER_UNIT],
                              capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    lines = done.stdout.split()
    return lines[1] if len(lines) == 2 and lines[0] == "active" else None


def windows_main():
    """yantrik-live-windows.service, as the desktop's account: Mind View on the left, the
    showcase on the right, again whenever the browser starts and when a mission ends (then the
    showcase in front: the games are the thing to watch)."""
    placed_for, place_at, was_live = None, 0.0, True
    while True:
        # As in showcase_main: SHOWING is the Mind's file, and one bad read or step is logged,
        # never the end of the loop.
        try:
            started = browser_started()
            if started is not None and started != placed_for:
                placed_for, place_at = started, time.time() + 10
            showing = read_state(SHOWING, {})
            try:
                live = bool(showing.get("mission")) and time.time() - float(showing.get("at") or 0) < SHOWING_STALE_SECS
            except (TypeError, ValueError):
                live = False
            if not live and was_live:
                place_at = place_at or time.time()
            if place_at and time.time() >= place_at:
                state = shell_state()
                # Never while a card is waiting: the shell holds window actions then, and the card
                # must stay where the person can see it (show_mind_view).
                if state is not None and not state.get("pending_approvals"):
                    place_windows(state, showcase_front=not live)
                    place_at = 0.0
            was_live = live
        except Exception as why:  # noqa: BLE001 - one bad step never stops the window loop
            log(event="window step failed", why=f"{type(why).__name__}: {why}"[:200])
        time.sleep(POLL_SECS)


def wayland_grant():
    """yantrik-live-showcase-grant.service, as root, before each start of the browser: let
    SHOWCASE_ACCOUNT connect to the desktop's Wayland socket, and only that socket. The socket is
    the desktop account's, 0755, so another account cannot connect; one ACL entry (rw for
    SHOWCASE_ACCOUNT) is added, set through the descriptor of what was opened, never a link. Its device and inode go to WAYLAND_RUN, root's,
    where showcase-browser checks that the socket bound into its unit is this one."""
    mind, show = pwd.getpwnam(MIND_ACCOUNT), pwd.getpwnam(SHOWCASE_ACCOUNT)
    path = f"/run/user/{mind.pw_uid}/{WAYLAND_SOCKET}"
    fd = os.open(path, os.O_PATH | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        st = os.fstat(fd)
        if not stat.S_ISSOCK(st.st_mode) or st.st_uid != mind.pw_uid:
            raise SystemExit(f"{path} is not the desktop's Wayland socket")
        undefined = 0xFFFFFFFF
        mode = st.st_mode
        # The ACL xattr: version 2, then (tag, perm, id) entries in tag order. USER_OBJ, the one
        # named user, GROUP_OBJ, the MASK (wide enough for that user, the group unchanged), OTHER.
        entries = [(0x01, (mode >> 6) & 7, undefined), (0x02, 6, show.pw_uid), (0x04, (mode >> 3) & 7, undefined),
                   (0x10, ((mode >> 3) & 7) | 6, undefined), (0x20, mode & 7, undefined)]
        acl = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", *e) for e in entries)
        os.setxattr(f"/proc/self/fd/{fd}", "system.posix_acl_access", acl)
        os.makedirs(WAYLAND_RUN, mode=0o755, exist_ok=True)
        run = os.lstat(WAYLAND_RUN)
        if not stat.S_ISDIR(run.st_mode) or run.st_uid != 0:
            raise SystemExit(f"{WAYLAND_RUN} is not root's")
        write_atomically(os.path.join(WAYLAND_RUN, "wayland.id"), f"{st.st_dev}:{st.st_ino}\n")
    finally:
        os.close(fd)


def load_builds():
    with open(os.path.join(HERE, "missions.json"), encoding="utf-8") as f:
        return [build(spec) for spec in json.load(f)]


def main(argv=()):
    command = list(argv[:1])
    if command == ["wayland-grant"]:
        wayland_grant()
        return 0
    if command == ["windows"]:
        windows_main()
        return 0
    builds = load_builds()
    if command == ["showcase"]:
        showcase_main(builds)
        return 0
    # Before anything is started: the key is given only while the Director is alone in its unit.
    why = become_director()
    if why:
        log(event="not started", why="not registered with the contracts service: " + why)
        return 0
    # systemctl stop mid-mission: SystemExit, so run_mission's finally tells the contract.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    m = next_mission(builds)
    if m is None:
        return 0
    try:
        run_mission(m)
    finally:
        # The mission is over, finished or not: the showcase goes back to the finished games.
        try:
            write_state(SHOWING, {})
        except OSError:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
