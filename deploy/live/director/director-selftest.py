#!/usr/bin/env python3
"""The Director's choices, with the desktop, the Mind and the page check stood in for.

Holds what the live machine depends on: a stopped build is resumed, not restarted; a FINISHED
over a page that throws is refused; the page's errors reach the Mind whole, braces and all; and
once every build is finished the Director goes round them in polish rounds instead of idling
(561 sat idle for two weeks after Starfall, 2 to 15 Oct 2026). Exit 0 when every check holds.
"""
import html.parser
import importlib.util
import json
import os
import re
import signal
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("director", os.path.join(HERE, "director.py"))
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)

tmp = tempfile.mkdtemp()
d.STATE_DIR = tmp
for k in ("LOG", "DONE", "STARTED", "POLISHED", "POLISH_DUE", "LAST", "SHOWING", "PARKED"):
    setattr(d, k, os.path.join(tmp, os.path.basename(getattr(d, k))))
d.PROJECTS = os.path.join(tmp, "Projects")
d.SHOWCASE_DIR = os.path.join(tmp, "showcase")
# Builds in order first; interleaving with polish rounds has its own checks further down.
d.POLISH_PER_BUILD = 0
d.BETWEEN_TURNS_SECS = 0
d.time.sleep = lambda s: None
d.show_mind_view = lambda: None
real_shell_state = d.shell_state
d.shell_state = lambda: {"thinking": False}
d.print = lambda *a, **k: None
sent, replies, pages = [], [], []
d.send = lambda text: (sent.append(text), (True, ""))[1]
d.wait_for_turn = lambda _: ({"conversation": [{"role": "assistant", "text": replies[0][0]}]}, replies[0][1],
                              replies.pop(0)[2:] == ("card",))
d.check_page = lambda m: pages.pop(0) if pages else (True, "")
# Every turn writes something, unless a check says the Mind wrote nothing.
real_folder_print = d.folder_print
prints, scripted = [0], []
d.folder_print = lambda m: scripted.pop(0) if scripted else (prints.__setitem__(0, prints[0] + 1), prints[0])[1]
chats = []
d.new_chat = lambda: (chats.append(len(sent)), (True, ""))[1]
# The contracts service, in process: the real store (contracts.py) on a scratch file, called as
# the kernel would name the Director.
cspec = importlib.util.spec_from_file_location("contracts", os.path.join(HERE, "contracts.py"))
c = importlib.util.module_from_spec(cspec)
cspec.loader.exec_module(c)
store = c.connect(os.path.join(tmp, "contracts.db"))
real_contract_call = d.contract_call
d.contract_call = lambda req: c.handle(store, "director", json.dumps(req).encode())
# Registering for this run's key is checked on its own, near the end.
real_become_director, d.become_director = d.become_director, lambda: None

failed = 0


def unit_lines(name):
    with open(os.path.join(HERE, name), encoding="utf-8") as f:
        return [line.strip() for line in f]


def check(name, ok, said=""):
    global failed
    print(("ok    " if ok else "FAIL  ") + name + ("" if ok else ": " + said[:300]))
    failed += not ok


with open(os.path.join(HERE, "missions.json"), encoding="utf-8") as f:
    specs = json.load(f)
check("every mission has the fields its wording needs",
      all({"id", "title", "name", "folder", "what", "details", "plan"} <= set(s) for s in specs))
check("mission ids and folders are unique",
      len({s["id"] for s in specs}) == len(specs) == len({s["folder"] for s in specs}))

d.write_state(d.DONE, {specs[0]["id"]})
second = specs[1]
replies[:] = [("working on it", "card unanswered")]
d.main()
check("the first unfinished build starts with its brief",
      sent[-1].startswith("Please build") and second["name"] in sent[-1] and "{" not in sent[-1], sent[-1])

replies[:] = [("still nothing", "card unanswered")]
d.main()
check("a stopped build with no plan yet is started again with its brief, not keep going",
      sent[-1].startswith("Please build") and second["name"] in sent[-1], sent[-1])

os.makedirs(os.path.join(d.PROJECTS, second["folder"]))
open(os.path.join(d.PROJECTS, second["folder"], "PLAN.md"), "w").close()
replies[:] = [("step done", "answered"), ("FINISHED", "answered")]
pages[:] = [(False, "Uncaught TypeError: x is {not} a function"), (True, "")]
d.main()
check("a stopped build with a plan is resumed with keep going", sent[-2].startswith("Please keep going on " + second["name"]), sent[-2])
check("a page that throws puts its errors, braces and all, at the head of the next prompt",
      "x is {not} a function" in sent[-1] and "PLAN.md side by side" in sent[-1], sent[-1])
check("a FINISHED over a clean page finishes the build", second["id"] in d.finished_missions())

replies[:] = [("FINISHED", "answered"), ("FINISHED", "answered")]
pages[:] = [(False, "SyntaxError: Unexpected token"), (True, "")]
before = len(sent)
d.main()
check("a FINISHED over a page that throws is refused, and the mission goes on",
      len(sent) - before == 2 and "SyntaxError" in sent[-1] and specs[2]["id"] in d.finished_missions())

d.write_state(d.DONE, {s["id"] for s in specs})
names = []
for _ in range(len(specs) + 2):
    replies[:] = [("FINISHED", "answered")]
    d.main()
    names.append(next(s["name"] for s in specs if sent[-1].startswith("Please make " + s["name"] + " better")))
check("with every build finished, polish rounds go round every build in turn",
      names == [s["name"] for s in specs] + [specs[0]["name"], specs[1]["name"]], repr(names))
check("a polish round works from IDEAS.md", "IDEAS.md" in sent[-1] and "{" not in sent[-1], sent[-1])
check("a polish round is never recorded as a finished mission",
      not any(m.startswith("polish:") for m in d.finished_missions()))

check("FINISHED counts on a line of its own, bare or in markdown, never inside a sentence",
      d.said_finished("All done.\n\nFINISHED") and d.said_finished("**FINISHED**")
      and not d.said_finished("Orrery is finished, every step is ticked") and not d.said_finished("UNFINISHED")
      and not d.said_finished("So nothing is finished, and I'm not going to write FINISHED on a line for work that isn't there."))

# A reply past describe's clip is read whole with read_message, where its FINISHED is.
calls = []


class Done:
    def __init__(self, stdout):
        self.stdout, self.returncode = stdout, 0


def fake_run(argv, **_):
    calls.append(argv)
    whole = json.dumps({"index": 9, "text": "x" * 700 + chr(10) + chr(10) + "FINISHED"}, indent=2)
    return Done("Yantrik, desktop\naccepted: True\n" + whole + "\n{\"state\": 1}\n")


real_run, d.subprocess.run = d.subprocess.run, fake_run
whole = d.whole_text({"index": 9, "text": "x" * 600 + "… (709 characters total)", "clipped": True})
d.subprocess.run = real_run
check("a clipped reply is read whole, so a FINISHED past the clip is seen",
      d.said_finished(whole) and calls and calls[0][-2:] == ["read_message", "index=9"], whole[-40:])
check("an unclipped reply is taken as it is, with no extra call",
      d.whole_text({"index": 3, "text": "short FINISHED"}) == "short FINISHED")

d.write_state(d.DONE, set())
replies[:] = [("the command needs your OK", "answered", "card"), ("still waiting on you", "answered", "card"),
              ("unused", "answered")]
before = len(sent)
d.main()
check("two turns running with a card stop the mission for a person",
      len(sent) - before == 2 and len(replies) == 1 and specs[0]["id"] not in d.finished_missions())
replies[:] = [("asked, then allowed", "answered", "card"), ("FINISHED", "answered")]
d.main()
check("one card that a person answers does not stop the mission", specs[0]["id"] in d.finished_missions())

d.write_state(d.DONE, set())
d.write_state(d.STARTED, set())
replies[:] = [("FINISHED", "answered")]
chats.clear()
before = len(sent)
d.main()
check("a mission started from its brief gets a fresh conversation first",
      chats == [before], repr(chats))
os.makedirs(os.path.join(d.PROJECTS, specs[1]["folder"]), exist_ok=True)
open(os.path.join(d.PROJECTS, specs[1]["folder"], "PLAN.md"), "w").close()
d.write_state(d.STARTED, {specs[1]["id"]})
d.write_state(d.LAST, specs[1]["id"])
replies[:] = [("FINISHED", "answered")]
chats.clear()
d.main()
check("a build resumed straight after itself keeps its conversation",
      chats == [] and sent[-1].startswith("Please keep going"), repr(chats))
d.write_state(d.DONE, set())
d.write_state(d.STARTED, {specs[0]["id"]})
os.makedirs(os.path.join(d.PROJECTS, specs[0]["folder"]), exist_ok=True)
open(os.path.join(d.PROJECTS, specs[0]["folder"], "PLAN.md"), "w").close()
d.write_state(d.LAST, "polish:" + specs[1]["id"])
replies[:] = [("FINISHED", "answered")]
chats.clear()
before = len(sent)
d.main()
check("a build resumed after another mission gets a fresh conversation and is told again what it is",
      chats == [before] and sent[-1].startswith("Please pick " + specs[0]["name"] + " up again")
      and specs[0]["details"] in sent[-1] and "{" not in sent[-1], sent[-1])

d.write_state(d.DONE, set())
d.write_state(d.STARTED, set())
replies[:] = [("I can't make progress: every command needs your approval.", "answered"),
              ("I still can't make progress: every command needs your approval.", "answered"),
              ("unused", "answered")]
before = len(sent)
d.main()
log = [json.loads(line) for line in open(d.LOG, encoding="utf-8")]
check("the same answer twice stops the mission",
      len(sent) - before == 2 and len(replies) == 1 and log[-1].get("why") == "the same answer twice", repr(log[-1]))
check("answers that differ are not taken for a repeat",
      not d.same_answer("Step 1 done: layers.js written.", "Step 2 done: the palette now saves.")
      and not d.same_answer("", ""))

d.write_state(d.DONE, set())
d.write_state(d.STARTED, set())
scripted[:] = [-1, -1, -2]  # at the start, after turn 1 (nothing written), after turn 2
replies[:] = [("FINISHED\n\nI wrote the save/load code and the check came back clean.", "answered"),
              ("Done for real this time.\n\nFINISHED", "answered")]
before = len(sent)
d.main()
log = [json.loads(line) for line in open(d.LOG, encoding="utf-8")]
refused = [e for e in log if e.get("event") == "finished refused" and e.get("why") == "nothing in the folder changed"]
check("a FINISHED over a folder nothing was written to is refused, and the Mind is told so",
      refused and len(sent) - before == 2 and "none of its code changed" in sent[-1], sent[-1])
check("a FINISHED once something was written finishes the round", specs[0]["id"] in d.finished_missions())

root = os.path.join(d.PROJECTS, "printcheck")
os.makedirs(os.path.join(root, ".git"), exist_ok=True)
m = {"fields": {"folder": "printcheck"}}
open(os.path.join(root, "a.js"), "w").write("1")
first = real_folder_print(m)
open(os.path.join(root, ".git", "x"), "w").write("hidden")
same = real_folder_print(m) == first
open(os.path.join(root, "IDEAS.md"), "w").write("- [x] done")
open(os.path.join(root, "a.js.bak"), "w").write("1")
open(os.path.join(root, "a.js"), "w").write("1")
notes_only = real_folder_print(m) == first
open(os.path.join(root, "a.js"), "w").write("12")
check("the folder print changes when code is written, not for notes, backups, rewrites or hidden entries",
      same and notes_only and real_folder_print(m) != first
      and real_folder_print({"fields": {"folder": "missing"}}) == ())
# The folder is the Mind's: a FIFO, a link to /dev/zero and a dangling link named like code are
# skipped, not opened (a FIFO would block the Director, /dev/zero would read without end).
before_hostile = real_folder_print(m)
os.mkfifo(os.path.join(root, "pipe.js"))
os.symlink("/dev/zero", os.path.join(root, "zero.js"))
os.symlink("/nonexistent", os.path.join(root, "gone.js"))
os.mkfifo(os.path.join(root, "IDEAS-fifo"))
signal.alarm(20)
hostile_print = real_folder_print(m)
ideas_fifo = os.path.join(d.PROJECTS, "fifoideas")
os.makedirs(ideas_fifo)
os.mkfifo(os.path.join(ideas_fifo, "IDEAS.md"))
fifo_demo = d.ensure_demo_idea({"fields": {"folder": "fifoideas"}})
signal.alarm(0)
check("the folder print skips FIFOs and links, and IDEAS.md as a FIFO is left alone, without blocking",
      hostile_print == before_hostile and fifo_demo is False, repr(hostile_print))

# ── The showcase ──────────────────────────────────────────────────────────────────────────────


class Page(html.parser.HTMLParser):
    """Every tag, every attribute that loads something, and the text, of a frame page."""

    def __init__(self):
        super().__init__()
        self.tags, self.loads, self.handlers, self.text = [], [], [], []

    def handle_starttag(self, tag, attrs):
        self.tags.append(tag)
        for k, v in attrs:
            if k in ("src", "href", "action", "data", "poster"):
                self.loads.append((tag, v))
            if k.startswith("on"):
                self.handlers.append((tag, k, v))

    def handle_data(self, data):
        self.text.append(data)


def frame():
    with open(os.path.join(d.SHOWCASE_DIR, "frame.html"), encoding="utf-8") as f:
        page = Page()
        page.feed(f.read())
    return page


def copied(url):
    """The copy a frame's game URL points at, as a directory, or None when it is not in GAMES_DIR."""
    prefix = "file://" + d.GAMES_DIR + "/"
    if not url.startswith(prefix) or not url.endswith("/index.html?demo=1"):
        return None
    return os.path.dirname(url[len("file://"):].split("?")[0])


me = os.getuid()
d.GAMES_DIR = os.path.join(d.SHOWCASE_DIR, "games")
os.makedirs(d.SHOWCASE_DIR, exist_ok=True)
d.ROTATION = os.path.join(tmp, "showcase-rotation")
d.ROTATION_OWNER = me
d.DIRECTOR_CGROUP = os.path.join(tmp, "cgroup.procs")
TEMPLATE_TAGS = {"html", "head", "meta", "title", "style", "body", "header", "div", "span", "b", "p", "script",
                 "iframe"}
os.makedirs(os.path.join(d.PROJECTS, specs[0]["folder"]), exist_ok=True)
open(os.path.join(d.PROJECTS, specs[0]["folder"], "index.html"), "w").write("<!doctype html><title>game</title>")
hostile = 'Done: <img src=x onerror="alert(1)"> it\'s "quoted" & <script>alert(2)</script></header><b>'
d.write_frame('Star"fall <i>x</i>', "turn 3 <b>", hostile, specs[0]["folder"], me)
page = frame()
text = "".join(page.text)
check("the frame page shows a reply with HTML and quotes as text, never as markup",
      set(page.tags) <= TEMPLATE_TAGS and page.tags.count("iframe") == 1 and "img" not in page.tags
      and page.tags.count("script") == 1 and page.handlers == [("body", "oncontextmenu", "return false")]
      and '<img src=x onerror="alert(1)">' in text and 'Star"fall <i>x</i>' in text and "turn 3 <b>" in text,
      repr((page.tags, page.handlers)))
shown_copy = copied(page.loads[0][1]) if page.loads else None
check("the frame page loads a root-held copy of the game's index.html, in its demo mode, nothing else",
      len(page.loads) == 1 and page.loads[0][0] == "iframe" and shown_copy is not None
      and os.path.basename(shown_copy).startswith(specs[0]["folder"] + "-")
      and open(os.path.join(shown_copy, "index.html")).read() == "<!doctype html><title>game</title>", repr(page.loads))
with open(os.path.join(d.SHOWCASE_DIR, "frame.html"), encoding="utf-8") as f:
    raw = f.read()
version = open(os.path.join(d.SHOWCASE_DIR, "version.js"), encoding="utf-8").read()
check("the frame page reloads itself when the publisher writes a new version",
      re.fullmatch(r'showcaseVersion\("[0-9a-f]{16}"\);\n', version) is not None
      and version.split('"')[1] in raw and '"version.js?"' in raw, version)
director_src = open(os.path.join(HERE, "director.py"), encoding="utf-8").read()
check("the game runs sandboxed: scripts and its own storage, no downloads, pop-ups, dialogs or navigating away;"
      " why allow-same-origin is safe is written beside it",
      'sandbox="allow-scripts allow-same-origin"' in raw and "Content-Security-Policy" in raw
      and "Never add --allow-file-access-from-files" in director_src)
d.write_frame("Missing", "turn 1", "", "no-such-game", me)
check("a game with no page yet shows a line instead of a frame", "iframe" not in frame().tags)

check("the caption is the reply's first sentence, plain text, with markup and paths taken out",
      d.caption_summary("## Step 3\n\n**Done:** I wrote the `collision` code in ~/Projects/starfall/game.js. "
                        "Next comes the score.\n\nFINISHED") == "Step 3 Done: I wrote the collision code in."
      and d.caption_summary("- [x] Added <b>sound</b> &amp; [a mute button](http://x/y). Then more.")
      == "Added sound & a mute button."
      and d.caption_summary("```js\nlet x = 1;\n```\nThe page loads clean now!") == "The page loads clean now!"
      and d.caption_summary("FINISHED") == "" and d.caption_summary("") == "",
      repr([d.caption_summary("## Step 3\n\n**Done:** I wrote the `collision` code in ~/Projects/starfall/game.js.")]))
long = d.caption_summary("word " * 80 + ".")
check("the caption is never longer than 120 characters", len(long) <= 120 and long.endswith("…"), repr(long))

# Nothing but a copy of ~/Projects/<mission folder> is ever put in the showcase.
outside = os.path.join(tmp, "private.html")
open(outside, "w").write("secret")
os.makedirs(os.path.join(d.PROJECTS, "linked"), exist_ok=True)
os.symlink(outside, os.path.join(d.PROJECTS, "linked", "index.html"))
os.symlink(tmp, os.path.join(d.PROJECTS, "via-link"))
os.makedirs(os.path.join(tmp, "escaped"), exist_ok=True)
open(os.path.join(tmp, "escaped", "index.html"), "w").write("x")
refused = []
for f in ("../escaped", "..", "/etc", "linked", "via-link", "Starfall", "a/b", "", None, "x" * 80):
    try:
        d.copy_game(f, me, "0")
        refused.append(f)
    except d.Refused:
        pass
check("the showcase copies nothing outside ~/Projects/<mission folder>: no climbing out, no links, no index.html"
      " that is one", refused == [], repr(refused))
for bad in ("../escaped", "linked", "via-link"):
    d.write_frame("x", "y", "z", bad, me)
    check(f"a frame for the folder {bad!r} loads nothing", frame().loads == [], repr(frame().loads))
try:
    d.copy_game(specs[0]["folder"], me + 1, "0")
    check("a folder the Mind's account does not own is not copied", False)
except d.Refused:
    check("a folder the Mind's account does not own is not copied", True)
check("every mission's folder is one the showcase can copy",
      all(d.FOLDER.match(s["folder"]) for s in specs), repr([s["folder"] for s in specs]))

# Inside a game: regular files and folders come over; links, hard links, pipes and hidden entries do not.
g = os.path.join(d.PROJECTS, "hostile")
os.makedirs(os.path.join(g, "assets", "deep"), exist_ok=True)
open(os.path.join(g, "index.html"), "w").write("<iframe src='file:///etc/passwd'></iframe>")
open(os.path.join(g, "assets", "deep", "a.png"), "w").write("png")
os.symlink(outside, os.path.join(g, "credits.html"))
os.symlink("/etc", os.path.join(g, "etc"))
os.link(outside, os.path.join(g, "hard.html"))
os.mkfifo(os.path.join(g, "pipe"))
os.makedirs(os.path.join(g, ".git"), exist_ok=True)
open(os.path.join(g, ".git", "config"), "w").write("x")
if me == 0:
    open(os.path.join(g, "theirs.js"), "w").write("not the Mind's")
    os.chown(os.path.join(g, "theirs.js"), 12345, 12345)
copy = os.path.dirname(d.copy_game("hostile", me, "abc"))
got = sorted(os.path.relpath(os.path.join(r, n), copy) for r, ds, fs in os.walk(copy) for n in ds + fs)
links = [p for p in got if os.path.islink(os.path.join(copy, p))]
check("a copied game holds only its own plain files and folders: no link, hard link, pipe, hidden or"
      " foreign file comes over", got == ["assets", "assets/deep", "assets/deep/a.png", "index.html"] and links == []
      and all(os.stat(os.path.join(copy, p)).st_uid == me for p in got), repr(got))
os.remove(os.path.join(g, "hard.html"))
real_limits = d.COPY_MAX_FILES, d.COPY_MAX_BYTES, d.COPY_MAX_DEPTH
over = []
real_entries = d.COPY_MAX_ENTRIES
for limits in ((2, 1 << 20, 6, 100), (100, 10, 6, 100), (100, 1 << 20, 2, 100), (100, 1 << 20, 6, 5)):
    d.COPY_MAX_FILES, d.COPY_MAX_BYTES, d.COPY_MAX_DEPTH, d.COPY_MAX_ENTRIES = limits
    try:
        d.copy_game("hostile", me, "big")
        over.append(limits)
    except d.Refused:
        pass
d.COPY_MAX_FILES, d.COPY_MAX_BYTES, d.COPY_MAX_DEPTH = real_limits
d.COPY_MAX_ENTRIES = real_entries
check("a game over the file, entry, byte or depth limit is not copied at all, and leaves no half copy behind",
      over == [] and not any("big" in n for n in os.listdir(d.GAMES_DIR)), repr((over, os.listdir(d.GAMES_DIR))))
for _ in range(4):
    d.write_frame("x", "y", "z", specs[0]["folder"], me)
check("only the copies a frame may still show are kept",
      len([n for n in os.listdir(d.GAMES_DIR)]) == d.GAMES_KEPT
      and copied(frame().loads[0][1]) in [os.path.join(d.GAMES_DIR, n) for n in os.listdir(d.GAMES_DIR)],
      repr(os.listdir(d.GAMES_DIR)))

with open(os.path.join(HERE, "showcase-browser"), encoding="utf-8") as f:
    browser = f.read()
# What it runs, its comments left out.
browser = "\n".join(line for line in browser.splitlines() if not line.lstrip().startswith("#"))
flags = re.findall(r"--[a-z-]+", browser)
check("the showcase browser opens only the frame page it is given",
      browser.count("--app=") == 1 and '--app="file://$frame"' in browser and "http" not in
      browser.replace('--proxy-server="http://127.0.0.1:9"', "") and d.SHOWCASE_BROWSER.endswith("showcase-browser"))
check("the showcase browser has no network, a throwaway profile (its HOME too), no extensions, no debugging"
      " port, and no file access between files",
      {"--proxy-server", "--host-resolver-rules", "--proxy-bypass-list", "--user-data-dir", "--disable-extensions",
       "--disable-component-extensions-with-background-pages", "--disable-background-networking"} <= set(flags)
      and '"MAP * ~NOTFOUND"' in browser and "mktemp -d" in browser and 'rm -rf "$profile"' in browser
      and 'export HOME="$profile"' in browser and "remote-debugging" not in browser
      and "load-extension" not in browser and "allow-file-access" not in browser)
check("the showcase browser starts only on the Wayland socket wayland-grant checked",
      'stat -c %d:%i "$sock"' in browser and "wayland.id" in browser)
check("the showcase browser puts no infobar or first-run prompt on the public stream",
      {"--test-type", "--no-first-run", "--no-default-browser-check", "--deny-permission-prompts"} <= set(flags))
publisher_unit = unit_lines("yantrik-live-showcase.service")
browser_unit = unit_lines("yantrik-live-showcase-browser.service")
windows_unit = unit_lines("yantrik-live-windows.service")
grant_unit = unit_lines("yantrik-live-showcase-grant.service")


def setting(unit, key):
    return " ".join(line.split("=", 1)[1] for line in unit if line.startswith(key + "="))


check("the publisher is root with one capability, reading the Mind's files: it can write the showcase and nothing"
      " else, sees no other home, and has no network",
      not any(line.startswith("User=") for line in publisher_unit)
      and setting(publisher_unit, "CapabilityBoundingSet") == "CAP_DAC_READ_SEARCH"
      and setting(publisher_unit, "ReadWritePaths") == "/opt/yantrik-live/showcase" and "ProtectSystem=strict" in publisher_unit
      and "ProtectHome=tmpfs" in publisher_unit
      and setting(publisher_unit, "BindReadOnlyPaths") == "-/home/yantrik/Projects -/home/yantrik/director"
      and {"NoNewPrivileges=yes", "PrivateNetwork=yes", "IPAddressDeny=any"} <= set(publisher_unit)
      and "ExecStart=/usr/bin/python3 -I /opt/yantrik-live/director/director.py showcase" in publisher_unit)
hidden = setting(browser_unit, "TemporaryFileSystem").split() + setting(browser_unit, "InaccessiblePaths").split()
check("the browser is its own account, with no capability, and can name nothing on disk but the published games",
      "User=yantrik-showcase" in browser_unit and "ProtectHome=yes" in browser_unit
      and {"/var", "/opt", "-/etc/yantrik", "-/etc/yantrik-live"} <= set(hidden)
      and setting(browser_unit, "BindReadOnlyPaths").split() == [
          "/opt/yantrik-live/showcase", "/opt/yantrik-live/director/showcase-browser",
          "/run/user/1000/wayland-0:/run/yantrik-live-showcase/wayland-0"]
      and not any(line.startswith(("BindPaths", "ReadWritePaths")) for line in browser_unit)
      and {"ProtectSystem=strict", "ProtectProc=invisible", "NoNewPrivileges=yes", "CapabilityBoundingSet=",
           "PrivateTmp=yes", "IPAddressDeny=any"} <= set(browser_unit)
      and not any(x.startswith("IPAddressAllow") for x in browser_unit + publisher_unit + windows_unit)
      and not any(line.startswith("ExecStartPre") for line in browser_unit))
# A "+" ExecStartPre runs inside the browser's namespace, where director.py and /run/user are hidden
# (live test on 561), so the grant is a root oneshot of its own. Not RemainAfterExit, so each start
# of the browser, every Restart=always one included, runs it again; Wants=, so a failed grant does
# not leave the browser waiting on its dependency for good (both seen on systemd 255).
check("the Wayland grant is a root oneshot outside the browser's namespace, run again before every browser start",
      "Type=oneshot" in grant_unit and not any(line.startswith(("User=", "RemainAfterExit", "ProtectHome",
                                                                "TemporaryFileSystem", "PrivateTmp")) for line in grant_unit)
      and setting(grant_unit, "ExecStart") == "/usr/bin/python3 -I /opt/yantrik-live/director/director.py wayland-grant"
      and setting(browser_unit, "Wants") == "yantrik-live-showcase-grant.service"
      and "yantrik-live-showcase-grant.service" in setting(browser_unit, "After").split()
      and not any(line.startswith(("Requires", "BindsTo")) for line in browser_unit))
check("the window loop is the desktop's account and runs no browser",
      "User=yantrik" in windows_unit and "IPAddressDeny=any" in windows_unit
      and setting(windows_unit, "ExecStart") == "/usr/bin/python3 /opt/yantrik-live/director/director.py windows"
      and "showcase-browser" not in setting(windows_unit, "ExecStart"))
setup = open(os.path.join(HERE, "setup-director.sh"), encoding="utf-8").read()
check("setup makes the showcase root's, the browser's account, and the root-only rotation, and installs all four"
      " units", "install -d -o root -g root -m 755 /opt/yantrik-live/showcase" in setup
      and "-o yantrik -g yantrik -m 755 /opt/yantrik-live/showcase" not in setup
      and "useradd --system" in setup and "yantrik-showcase" in setup and "/etc/yantrik-live/showcase-rotation" in setup
      and all(u in setup for u in ("yantrik-live-showcase.service", "yantrik-live-showcase-browser.service",
                                   "yantrik-live-showcase-grant.service", "yantrik-live-windows.service")))

# While a mission runs, the Director asks for its game; between missions the publisher goes round the rotation.
d.write_state(d.DONE, set())
d.write_state(d.STARTED, set())
seen = []
real_show = d.show_mission


def watching_show(m, turn=0, reply="", clean=None):
    real_show(m, turn, reply, clean)
    seen.append(d.read_state(d.SHOWING, {}))


d.show_mission = watching_show
replies[:] = [("Step one is in. More to come.", "answered"), ("FINISHED", "answered")]
d.main()
d.show_mission = real_show
check("the Director asks for the running mission's game after every turn, with its caption, and writes no frame",
      [(x["mission"], x["turn"]) for x in seen] == [(specs[0]["id"], t) for t in (0, 1, 2)]
      and seen[1]["summary"] == "Step one is in." and "showcase not" not in open(d.LOG).read(),
      repr(seen))
check("once the mission ends the showcase is handed back to the rotation", d.read_state(d.SHOWING, {}) == {})

builds = d.load_builds()
open(d.DIRECTOR_CGROUP, "w").write("4242\n")
d.write_state(d.SHOWING, {"mission": "polish:" + specs[0]["id"], "at": d.time.time(), "turn": 3, "clean": False,
                          "summary": "Added <b>sound</b>. More."})
live = d.live_request(builds, me)
check("the publisher shows the Director's mission, with a turn line it makes and the caption made plain again",
      live is not None and live[0]["id"] == "polish:" + specs[0]["id"]
      and live[1] == "turn 3 · the page throws an error, being fixed" and live[2] == "Added sound.", repr(live))
check("the commands of the turn just ended are read from what the shell showed, after the Director's message",
      d.turn_calls({"conversation": [{"role": "assistant", "calls": [{"name": "old", "summary": "ls"}]},
                                     {"role": "user", "text": "keep going"},
                                     {"role": "assistant", "calls": [{"name": "shell.agent_run", "summary": "mkdir -p\n x"},
                                                                     "not a call"]}]})
      == ["shell.agent_run: mkdir -p x"])
forged = []
for req in ({"mission": "../etc", "at": d.time.time()}, {"mission": specs[0]["id"], "at": 0},
            {"mission": specs[0]["id"], "at": "soon"}, ["not", "a", "dict"]):
    open(d.SHOWING, "w").write(json.dumps(req))
    if d.live_request(builds, me) is not None:
        forged.append(req)
d.write_state(d.SHOWING, {"mission": specs[0]["id"], "at": d.time.time(), "turn": "<b>9</b>"})
turn_line = d.live_request(builds, me)[1]
open(d.SHOWING, "w").write(json.dumps({"mission": specs[0]["id"], "at": d.time.time(), "summary": "x" * 20000}))
oversize = d.live_request(builds, me)
os.remove(d.SHOWING)
os.symlink(os.path.join(tmp, "elsewhere.json"), d.SHOWING)
open(os.path.join(tmp, "elsewhere.json"), "w").write(json.dumps({"mission": specs[0]["id"], "at": d.time.time()}))
linked = d.live_request(builds, me)
os.remove(d.SHOWING)
d.write_state(d.SHOWING, {"mission": specs[0]["id"], "at": d.time.time()})
open(d.DIRECTOR_CGROUP, "w").write("")
idle = d.live_request(builds, me)
check("a request for no mission, a stale one, an oversized one, one through a link, or one while the Director"
      " is not running shows nothing; a forged turn is no turn",
      forged == [] and turn_line == "starting" and oversize is None and linked is None and idle is None,
      repr((forged, turn_line, oversize, linked, idle)))

# The request is the Mind's file: whatever it holds, the publisher reads it as no request and carries on.
# 16 KiB of "[" fits under REQUEST_MAX_BYTES and is deep enough to raise RecursionError in json (#840).
HOSTILE = {"16 KiB of [": b"[" * d.REQUEST_MAX_BYTES, "invalid UTF-8": b'{"mission": "\xff\xfe"}',
           "a 10 MB body": b'{"mission": "' + b"x" * (10 << 20) + b'"}', "a non-object": b'"starfall"',
           "a deeply nested object": b'{"a":' * 3000 + b"1" + b"}" * 3000}
open(d.DIRECTOR_CGROUP, "w").write("4242\n")
fell = {}
for name, body in HOSTILE.items():
    open(d.SHOWING, "wb").write(body)
    try:
        if d.live_request(builds, me) is not None:
            fell[name] = "shown"
    except Exception as why:
        fell[name] = type(why).__name__
check("a request of 16 KiB of [, invalid UTF-8, 10 MB, a non-object or deep nesting is no request, and raises nothing",
      fell == {}, repr(fell))
# A lone surrogate is valid JSON that no encoder takes: the caption drops it, so the frame can be written.
open(d.SHOWING, "w").write('{"mission": "%s", "at": %f, "turn": 2, "summary": "Turn done \\ud800 ok."}'
                           % (specs[0]["id"], d.time.time()))
surrogate = d.live_request(builds, me)
try:
    surrogate[2].encode("utf-8")
    encodes = True
except (TypeError, UnicodeEncodeError):
    encodes = False
check("a caption with a lone surrogate is shown without it, so writing the frame cannot fail on it",
      surrogate is not None and encodes and surrogate[2] == "Turn done ok.", repr(surrogate))
# Python 3.12 and later parse the 16 KiB nested object (it is valid JSON, and read_state may return it);
# 100,000 levels, under STATE_MAX_BYTES, raise RecursionError on every version.
fell = {}
for name, body in dict(HOSTILE, **{"a 100,000-deep object": b'{"a":' * 100000 + b"1" + b"}" * 100000}).items():
    open(d.SHOWING, "wb").write(body)
    try:
        got = d.read_state(d.SHOWING, {})
        if got != {} and not (name == "a deeply nested object" and list(got) == ["a"]):
            fell[name] = "read"
    except Exception as why:
        fell[name] = type(why).__name__
os.remove(d.SHOWING)
os.mkfifo(d.SHOWING)


def blocked(*_):
    raise TimeoutError("blocked on the FIFO")


signal.signal(signal.SIGALRM, blocked)
signal.alarm(5)
try:
    if d.read_state(d.SHOWING, {}) != {}:
        fell["a FIFO"] = "read"
except Exception as why:
    fell["a FIFO"] = type(why).__name__
signal.alarm(0)
os.remove(d.SHOWING)
check("the window loop's read of the same file is empty for each of them, and for a FIFO, and raises or blocks on"
      " nothing", fell == {}, repr(fell))
real_run = d.subprocess.run
fell = {}
for name, body in HOSTILE.items():
    d.subprocess.run = lambda argv, **k: Done("shell\n" + body.decode("utf-8", "replace").replace("[", "{", 1) + "\n}")
    try:
        d.shell_state()
        d.whole_text({"text": "t", "clipped": True, "index": 1})
        d.place_windows(None)
    except Exception as why:
        fell[name] = type(why).__name__
d.subprocess.run = real_run
check("the shell's describe output is parsed the same way, so no reply of the desktop's ends the Director's loops",
      fell == {}, repr(fell))
open(d.DIRECTOR_CGROUP, "w").write("")

for s in specs[:3]:
    os.makedirs(os.path.join(d.PROJECTS, s["folder"]), exist_ok=True)
    open(os.path.join(d.PROJECTS, s["folder"], "index.html"), "w").write("x")
# The Mind can write done.json; only the rotation counts. specs[3] has no page.
d.write_state(d.DONE, {s["id"] for s in specs})
open(d.ROTATION, "w").write(f"# games\n{specs[2]['id']}\n{specs[0]['id']}  # Starfall\n{specs[3]['id']}\nnot-a-mission\n")
os.chmod(d.ROTATION, 0o644)
shown, index = [], 0
for _ in range(3):
    index = d.cycle_step(builds, index, me)
    shown.append(os.path.basename(copied(frame().loads[0][1])).rsplit("-", 1)[0])
check("between missions the showcase goes round the games a person put in the rotation, with a page, in their"
      " demo mode", shown == [specs[0]["folder"], specs[2]["folder"], specs[0]["folder"]], repr(shown))
os.chmod(d.ROTATION, 0o666)
d.cycle_step(builds, 0, me)
writable = frame().loads
os.remove(d.ROTATION)
d.cycle_step(builds, 0, me)
check("a rotation anyone else could write counts as none, and with no rotation the showcase says so",
      writable == [] and frame().loads == [] and "No game is in the rotation yet" in "".join(frame().text))
check("the publisher, the browser and the window loop are services of their own, so the Director's oneshot"
      " still ends with its mission",
      all("Restart=always" in u for u in (publisher_unit, browser_unit, windows_unit))
      and "Type=oneshot" in unit_lines("yantrik-live-director.service"))

# The publisher's loop: the rotation until the Director runs a mission, then the mission, copied each turn at most.
class Stop(Exception):
    pass


open(d.ROTATION, "w").write(f"{specs[0]['id']}\n{specs[2]['id']}\n")
os.chmod(d.ROTATION, 0o644)
d.write_state(d.SHOWING, {})
open(d.DIRECTOR_CGROUP, "w").write("")
clock, ticks, frames = [1000.0], [0], []
real_time, d.time.time = d.time.time, lambda: clock[0]
real_sleep = d.time.sleep
real_pw = d.pwd.getpwnam
d.pwd.getpwnam = lambda name: type("pw", (), {"pw_uid": me})()


def tick(_):
    ticks[0] += 1
    header = re.search(r"<header>(.*?)</header>", open(os.path.join(d.SHOWCASE_DIR, "frame.html")).read()).group(1)
    frames.append(html.unescape(re.sub(r"<[^>]+>", "", header)))
    clock[0] += d.POLL_SECS if ticks[0] != 3 else d.CYCLE_SECS
    if ticks[0] == 5:
        open(d.DIRECTOR_CGROUP, "w").write("4242\n")
        d.write_state(d.SHOWING, {"mission": specs[1]["id"], "at": clock[0], "turn": 1})
    if ticks[0] == 7:
        # The Mind rewriting the request every poll: copied again only after LIVE_COPY_SECS.
        d.write_state(d.SHOWING, {"mission": specs[1]["id"], "at": clock[0], "turn": 2})
    if ticks[0] == 10:
        raise Stop


d.time.sleep = tick
try:
    d.main(["showcase"])
except Stop:
    pass
d.time.sleep, d.time.time, d.pwd.getpwnam = real_sleep, real_time, real_pw
d.LOG_TO_FILE[0] = True
mission_frames = [f for f in frames if f.startswith("LIVE" + specs[1]["title"])]
check("the publisher goes round the rotation until the Director runs a mission, then shows that mission,"
      " copied again no faster than LIVE_COPY_SECS",
      [f.startswith("LIVE" + specs[0]["title"]) for f in frames[:3]] == [True, True, True]
      and frames[3].startswith("LIVE" + specs[2]["title"]) and len(mission_frames) == 4
      and "turn 1" in mission_frames[0] and "turn 1" in mission_frames[1] and "turn 2" in mission_frames[-1],
      repr([f[4:12] + "|" + f[-20:] for f in frames]))

# One bad request, or any one failure in a step, never ends the publisher: on 0040a380 the 16 KiB of "["
# escaped showcase_main, Restart=always brought it back to the same file, and the showcase froze.
open(d.DIRECTOR_CGROUP, "w").write("4242\n")
clock, ticks, frames, broke = [1000.0], [0], [], [1]
d.time.time = lambda: clock[0]
d.pwd.getpwnam = lambda name: type("pw", (), {"pw_uid": me})()
real_frame = d.write_frame


def breaking_frame(*a, **k):
    if broke[0]:
        broke[0] = 0
        raise RuntimeError("one step failed")
    return real_frame(*a, **k)


def tick(_):
    ticks[0] += 1
    clock[0] += d.LIVE_COPY_SECS
    body = list(HOSTILE.values())[ticks[0] - 1] if ticks[0] <= len(HOSTILE) else None
    if body is not None:
        open(d.SHOWING, "wb").write(body)
    elif ticks[0] == len(HOSTILE) + 1:
        d.write_state(d.SHOWING, {"mission": specs[1]["id"], "at": clock[0], "turn": 4})
    else:
        header = re.search(r"<header>(.*?)</header>", open(os.path.join(d.SHOWCASE_DIR, "frame.html")).read())
        frames.append(html.unescape(re.sub(r"<[^>]+>", "", header.group(1))))
    if ticks[0] == len(HOSTILE) + 4:
        raise Stop


d.write_frame, d.time.sleep = breaking_frame, tick
real_log, logged = d.log, []
d.log = lambda **entry: logged.append(entry)
d.write_state(d.SHOWING, {"mission": specs[1]["id"], "at": clock[0], "turn": 3})
ended = None
try:
    d.showcase_main(builds)
except Stop:
    pass
except Exception as why:
    ended = type(why).__name__
d.time.sleep, d.time.time, d.pwd.getpwnam, d.write_frame = real_sleep, real_time, real_pw, real_frame
d.log = real_log
d.LOG_TO_FILE[0] = True
check("the publisher outlives a failing step and every hostile request, logs the failure, and shows the next good"
      " request", ended is None and bool(frames) and "turn 4" in frames[-1] and "one step failed" in repr(logged),
      repr((ended, frames, ticks[0], logged)))
open(d.DIRECTOR_CGROUP, "w").write("")

# The window loop: placed when the browser starts, and with the showcase in front when a mission ends.
placed, starts, ticks = [], ["a", "a", "a", "b", "b", "b", "b"], [0]
real_place, real_started = d.place_windows, d.browser_started
d.place_windows = lambda state, showcase_front=False: placed.append((ticks[0], showcase_front))
d.browser_started = lambda: starts[min(ticks[0], len(starts) - 1)]
d.write_state(d.SHOWING, {})
clock = [1000.0]
d.time.time = lambda: clock[0]


def tick(_):
    ticks[0] += 1
    clock[0] += 6
    if ticks[0] == 7:
        raise Stop


d.time.sleep = tick
try:
    d.main(["windows"])
except Stop:
    pass
d.time.sleep, d.time.time, d.place_windows, d.browser_started = real_sleep, real_time, real_place, real_started
check("the window loop places the windows once the browser starts and again when it restarts, never twice for one"
      " start", placed == [(2, True), (5, True)], repr(placed))

# A build that has stopped moving is parked, not resumed for hours (#863). Replies that share no
# opening, so "the same answer twice" never ends these missions first.
import hashlib
import sqlite3


def different(n):
    return [(hashlib.sha256(b"%d" % i).hexdigest() + " " + hashlib.sha256(b"x%d" % i).hexdigest(), "answered")
            for i in range(n)]


def clear_state():
    for k in (d.DONE, d.STARTED, d.POLISHED, d.POLISH_DUE, d.LAST, d.PARKED):
        if os.path.lexists(k):
            os.remove(k)


def log_since(mark):
    with open(d.LOG, encoding="utf-8") as f:
        return [json.loads(line) for line in f.read()[mark:].splitlines()]


def stopped_whys(contract_id):
    db = sqlite3.connect(os.path.join(tmp, "contracts.db"))
    try:
        return [json.loads(detail).get("why") for kind, detail in db.execute(
            "SELECT kind, detail FROM events WHERE contract = ? ORDER BY seq", (contract_id,)) if kind == "stopped"]
    finally:
        db.close()


clear_state()
captions = []


def caption_show(m, turn=0, reply="", clean=None):
    real_show(m, turn, reply, clean)
    captions.append(d.read_state(d.SHOWING, {}).get("summary"))


d.show_mission = caption_show
scripted[:] = [7] * (d.STALL_TURNS + 1)  # at the start and after each turn: nothing written
replies[:] = different(40)  # as many as the build's turn limit, so a build not parked runs to it
before, mark = len(sent), os.path.getsize(d.LOG)
d.main()
d.show_mission = real_show
log = log_since(mark)
parked_line = [e for e in log if e.get("event") == "parked"]
stop_line = [e for e in log if e.get("event") == "stopped"]
check("a build whose folder is unchanged for 8 turns running stops with 'no progress in 8 turns' and is parked "
      "for 6 hours, logged and on the contract",
      d.STALL_TURNS == 8 and d.PARK_HOURS == 6 and len(sent) - before == 8 and len(replies) == 32
      and parked_line and parked_line[0]["mission"] == specs[0]["id"] and parked_line[0]["stalled_turns"] == 8
      and parked_line[0].get("until")
      and stop_line and stop_line[-1]["why"] == "no progress in 8 turns"
      and stopped_whys(stop_line[-1]["contract"]) == ["no progress in 8 turns"]
      and 6 * 3600 - 60 < d.parks().get(specs[0]["id"], 0) - d.time.time() <= 6 * 3600,
      repr((len(sent) - before, parked_line, stop_line, d.parks())))
check("the stream's caption says the build is being parked, in plain words",
      captions[-1] == "Parking %s: 8 turns without a change to its files; back to it later." % specs[0]["name"],
      repr(captions[-1:]))

clear_state()
scripted[:] = [7] * 8 + [8, 8]  # 7 turns unchanged, then a change, then FINISHED over it
replies[:] = different(8) + [("FINISHED", "answered")]
mark = os.path.getsize(d.LOG)
d.main()
log = log_since(mark)
check("7 unchanged turns and then a change are not parked: the count starts again and the build finishes",
      not any(e.get("event") == "parked" for e in log) and d.parks() == {} and specs[0]["id"] in d.finished_missions(),
      repr(log[-3:]))

clear_state()
scripted[:] = [7] * 7 + [8] + [8] * 7 + [9]  # 7 unchanged, a change, 7 unchanged, a change
replies[:] = different(15) + [("FINISHED", "answered")]
mark = os.path.getsize(d.LOG)
d.main()
check("a build that changes something at least every 8th turn is never parked",
      not any(e.get("event") == "parked" for e in log_since(mark)) and specs[0]["id"] in d.finished_missions())

clear_state()
d.write_state(d.DONE, {s["id"] for s in specs})
scripted[:] = [7] * (d.POLISH_TURNS + 1)
replies[:] = different(d.POLISH_TURNS)
mark = os.path.getsize(d.LOG)
d.main()
log = log_since(mark)
check("a polish round keeps its own limits and is never parked",
      not any(e.get("event") == "parked" for e in log) and d.parks() == {}
      and [e["why"] for e in log if e.get("event") == "stopped"] == ["turn limit"], repr(log[-2:]))

builds = d.load_builds()
far = d.time.time() + 3600
clear_state()
d.write_state(d.PARKED, {specs[0]["id"]: far})
check("the next pick passes over a parked build for the next unfinished one",
      d.next_mission(builds)["id"] == specs[1]["id"])
clear_state()
d.write_state(d.DONE, {specs[0]["id"]})
d.write_state(d.PARKED, {s["id"]: far for s in specs[1:]})
mark = os.path.getsize(d.LOG)
picked = d.next_mission(builds)
check("with every unfinished build parked, a finished build gets a polish round",
      picked["id"] == "polish:" + specs[0]["id"]
      and any(e.get("event") == "every build is parked" for e in log_since(mark)), repr(picked["id"]))
clear_state()
d.write_state(d.PARKED, {s["id"]: far for s in specs})
mark = os.path.getsize(d.LOG)
picked = d.next_mission(builds)
check("with every build parked and none finished, nothing runs, logged 'every build is parked'",
      picked is None and {"event": "idle", "why": "every build is parked"}.items()
      <= (log_since(mark) or [{}])[-1].items(), repr(log_since(mark)))
replies[:] = different(40)
before = len(sent)
d.main()
check("and the Director exits without sending anything", len(sent) == before and len(replies) == 40)

clear_state()
os.makedirs(os.path.join(d.PROJECTS, specs[0]["folder"]), exist_ok=True)
open(os.path.join(d.PROJECTS, specs[0]["folder"], "PLAN.md"), "w").close()
d.write_state(d.STARTED, {specs[0]["id"]})
d.write_state(d.LAST, specs[1]["id"])
d.write_state(d.PARKED, {specs[0]["id"]: d.time.time() - 1})
replies[:] = [("FINISHED", "answered")]
d.main()
check("once its park is over the build is picked again, and resumed where it was, not started over",
      sent[-1].startswith("Please pick " + specs[0]["name"] + " up again") and d.read_state(d.PARKED, {}) == {},
      sent[-1])

# A person's choice wins. Nothing today lets a person pick the Director's next mission;
# whatever does goes through mission_for and run_mission, and run_mission runs a parked build and
# lifts its park. The showcase, which only shows what the Director runs, ignores parks.
clear_state()
d.write_state(d.PARKED, {specs[0]["id"]: far, specs[1]["id"]: far})
replies[:] = [("FINISHED", "answered")]
before = len(sent)
d.run_mission(d.mission_for(builds, specs[0]["id"]))
check("a mission a person names runs even while parked, and its park is lifted (the others' kept)",
      len(sent) - before == 1 and sent[-1].startswith("Please build") and specs[0]["name"] in sent[-1]
      and list(d.parks()) == [specs[1]["id"]], repr(d.parks()))
open(d.DIRECTOR_CGROUP, "w").write("4242\n")
d.write_state(d.SHOWING, {"mission": specs[1]["id"], "at": d.time.time(), "turn": 2, "summary": "Going."})
live = d.live_request(builds, me)
open(d.DIRECTOR_CGROUP, "w").write("")
check("a live request for a parked build is still shown", live is not None and live[0]["id"] == specs[1]["id"])

# The park file is in ~/director, which the Mind's account can write.
first = specs[0]["id"]
hostile = {
    "a list instead of a dict": json.dumps([first]),
    "an unknown id": json.dumps({"not-a-mission": 1e18}),
    "a string until": json.dumps({first: "99999999999"}),
    "a boolean until": json.dumps({first: True}),
    "NaN": '{"%s": NaN}' % first,
    "2 MB of JSON": json.dumps({first: 1e18, "pad": "x" * (2 << 20)}),
    "16 KiB of [": "[" * (16 << 10),
    "not JSON": "parked!",
}
fell = {}
for name, body in hostile.items():
    clear_state()
    open(d.PARKED, "w").write(body)
    try:
        if d.next_mission(builds)["id"] != first:
            fell[name] = "the build was passed over"
    except Exception as why:
        fell[name] = type(why).__name__
for name, until in (("until = 1e18", 1e18), ("until = 1e400", "1e400"), ("a 1000-digit until", "9" * 1000)):
    clear_state()
    open(d.PARKED, "w").write('{"%s": %s}' % (first, until))
    picked = d.next_mission(builds)["id"]
    cap = d.time.time() + d.PARK_HOURS * 3600
    # Clamped once and written back so: six hours later it is over, however often it was read.
    got = d.parks().get(first)
    if picked == first or got is None or got > cap or d.parks(cap + 1) != {}:
        fell[name] = repr((picked, got, d.parks(cap + 1)))
clear_state()
os.mkfifo(d.PARKED)
signal.signal(signal.SIGALRM, blocked)
signal.alarm(5)
try:
    if d.next_mission(builds)["id"] != first or d.parks() != {}:
        fell["a FIFO"] = "read"
except Exception as why:
    fell["a FIFO"] = type(why).__name__
signal.alarm(0)
clear_state()
check("a hostile park file parks nothing, or parks for at most 6 hours: a list, an unknown id, a string, boolean "
      "or NaN until, 2 MB of JSON, deep nesting, not JSON, a FIFO, until = 1e18, 1e400 or 1000 digits",
      fell == {}, repr(fell))

# Builds and polish rounds take turns.
d.POLISH_PER_BUILD = 2
for k in (d.DONE, d.STARTED, d.POLISHED, d.POLISH_DUE, d.LAST):
    if os.path.exists(k):
        os.remove(k)
order = []
for _ in range(8):
    replies[:] = [("FINISHED", "answered")]
    d.main()
    order.append(next(("polish " if sent[-1].startswith("Please make") else "build ") + s["id"] for s in specs
                      if re.search(r"\b%s\b" % re.escape(s["name"]), sent[-1])))
check("builds and polish rounds are interleaved, each finished build followed by its polish rounds",
      order == ["build " + specs[0]["id"], "polish " + specs[0]["id"], "polish " + specs[0]["id"],
                "build " + specs[1]["id"], "polish " + specs[1]["id"], "polish " + specs[0]["id"],
                "build " + specs[2]["id"], "polish " + specs[2]["id"]], repr(order))
d.POLISH_PER_BUILD = 0

# Every game's first polish idea is its demo mode.
check("every brief asks for the demo mode as the first idea in IDEAS.md",
      all(d.DEMO_IDEA in d.prompt(t, d.build(specs[0])) for t in (d.BRIEF, d.KEEP_GOING, d.RESUME))
      and "?demo=1" in d.DEMO_IDEA and "the first of them " + d.DEMO_IDEA in d.BRIEF)
ideas = os.path.join(d.PROJECTS, specs[0]["folder"], "IDEAS.md")
open(ideas, "w").write("# Ideas for Starfall\n\n- [x] Sound effects\n- [ ] Power-ups\n")
d.write_state(d.DONE, {s["id"] for s in specs})
replies[:] = [("FINISHED", "answered")]
d.write_state(d.POLISHED, {s["id"]: 1 for s in specs[1:]})
d.main()
got = open(ideas, encoding="utf-8").read().splitlines()
check("a polish round puts the demo idea at the head of IDEAS.md, unticked, ahead of the game's own ideas",
      got[:3] == ["# Ideas for Starfall", "", "- [ ] " + d.DEMO_IDEA[0].upper() + d.DEMO_IDEA[1:]]
      and got[3:] == ["- [x] Sound effects", "- [ ] Power-ups"] and sent[-1].startswith("Please make Starfall"),
      repr(got))
m0 = d.polish(d.build(specs[0]))
check("the demo idea is added once, and not over a demo the game already has",
      not d.ensure_demo_idea(m0) and open(ideas).read().count("demo=1") == 1)
open(ideas, "w").write("- [x] A demo mode on ?demo=1\n")
check("a game whose demo is ticked gets no second one", not d.ensure_demo_idea(m0))
os.remove(ideas)
check("a game with no IDEAS.md gets one, the demo first", d.ensure_demo_idea(m0)
      and open(ideas).read().startswith("- [ ] A self-playing demo mode"))
os.remove(ideas)
os.symlink(outside, ideas)
check("an IDEAS.md that is a link is left alone", not d.ensure_demo_idea(m0) and open(outside).read() == "secret")

# Mind View left, the showcase right, and the showcase never left behind Mind View.
view = {"actions": [{"name": "focus_window", "parameters": {"properties": {"title": {}}}},
                    {"name": "snap_window", "parameters": {"properties": {
                        "title": {"type": "string"}, "region": {"enum": ["left-half", "right-half", "top-half"]}}}}]}
snap = d.snap_action(view)
check("the shell's snap action is found from what it publishes, and its halves chosen",
      d.snap_args(snap, "Mind View", "left") == ["snap_window", "title=Mind View", "region=left-half"]
      and d.snap_args(snap, d.SHOWCASE_TITLE, "right") == ["snap_window", "title=Yantrik Live showcase", "region=right-half"]
      and d.snap_action({"actions": view["actions"][:1]}) is None, repr(snap))
acts, ran = [], []
real_act, d.act = d.act, lambda *a: (acts.append(a), (True, ""))[1]
d.subprocess.run = lambda argv, **k: (ran.append((argv, (k.get("env") or {}).get("WAYLAND_DISPLAY"))),
                                      Done(json.dumps(view)))[1]
d.place_windows({"mind_view": {"display": "wayland-1"}})
d.subprocess.run = real_run
check("Mind View is snapped left and the showcase right, and the showcase is brought forward last",
      acts == [("snap_window", "title=Mind View", "region=left-half"),
               ("snap_window", "title=Yantrik Live showcase", "region=right-half"),
               ("focus_window", "title=Yantrik Live showcase")], repr(acts))
check("the Mind's terminal fills Mind View, so the empty-view picture is not seen round it",
      ran[-1] == (["wlrctl", "toplevel", "maximize", "app_id:foot"], "wayland-1"), repr(ran))
acts.clear()
ran.clear()
d.subprocess.run = lambda argv, **k: (ran.append(argv), Done(json.dumps({"actions": [{"name": "taint_snapshot"}]})))[1]
snapped = d.place_windows({"mind_view": {"display": "../wayland-0"}})
d.subprocess.run = real_run
check("without a snap action Mind View stays in front during a mission, and no display but Mind View's is touched",
      not snapped and acts == [] and not any(a[0] == "wlrctl" for a in ran), repr((acts, ran)))
d.subprocess.run = lambda argv, **k: Done(json.dumps({"actions": []}))
d.place_windows(None, showcase_front=True)
d.subprocess.run = real_run
check("without a snap action the showcase is brought in front between missions",
      acts == [("focus_window", "title=Yantrik Live showcase")], repr(acts))
d.act = real_act

with open(os.path.join(HERE, "yantrik-live-director.service"), encoding="utf-8") as f:
    unit = [line.strip() for line in f]
check("the Director's service has no IP, so a page the Mind wrote reaches no network when checked",
      "IPAddressDeny=any" in unit and not any(line.startswith("IPAddressAllow") for line in unit))

# Task contracts (contracts.py): every mission is one, and the Director's FINISHED is delivered,
# never accepted.


def contract_for(mission):
    row = store.execute("SELECT id FROM contracts WHERE mission=? ORDER BY rowid DESC LIMIT 1", (mission,)).fetchone()
    return c.contract_view(store, row[0]) if row else None


def as_person(req):
    return c.handle(store, "person", json.dumps(req).encode())


first = specs[0]
d.write_state(d.DONE, set())
d.write_state(d.STARTED, set())
# A deliberately false FINISHED: the Mind touches one file (a meaningless edit) and says it built
# everything; the page loads clean. The Director's heuristic passes it, so it is delivered, and
# that is as far as it goes.
scripted[:] = [-10, -11]
replies[:] = [("All eight steps are in and working.\n\nFINISHED", "answered")]
d.main()
k = contract_for(first["id"])
check("a mission is a contract whose intent is its brief, with the standing rule recorded as legacy",
      k is not None and k["revisions"][0]["intent"]["brief"].startswith("Please build")
      and k["revisions"][0]["grant"]["status"] == "legacy, not contract-enforced", repr(k and k["revisions"][0]))
check("a false FINISHED over a folder that changed, with a clean page, stays delivered, not accepted",
      k["state"] == "delivered" and k["decision"] is None
      and [e["kind"] for e in k["events"]][-3:] == ["file_changed", "page_check", "finished_said"], repr(k["events"][-3:]))
forged = [c.handle(store, "director", json.dumps(r).encode()) for r in (
    {"op": "decide", "contract": k["id"], "verdict": "accepted", "note": "looks done"},
    {"op": "event", "contract": k["id"], "revision": 1, "kind": "accepted"},
    {"op": "event", "contract": k["id"], "revision": 1, "kind": "turn_started", "turn": 2, "prompt": "x"})]
check("nothing the Director can send accepts a contract or moves a delivered one on",
      not any(r["ok"] for r in forged) and contract_for(first["id"])["state"] == "delivered", repr(forged))
rejected = as_person({"op": "decide", "contract": k["id"], "verdict": "rejected", "note": "the levels are not there"})
k = contract_for(first["id"])
check("a person's rejection ends it, with the reason and the revision it applied to",
      rejected["ok"] and k["state"] == "failed" and k["decision"]["verdict"] == "rejected"
      and k["decision"]["revision"] == 1 and "levels" in k["reason"], repr((rejected, k["decision"])))

# A FINISHED over nothing written is refused and the contract runs on; the true one after it is
# delivered, and accepted only when a person says so.
second = specs[1]
d.write_state(d.DONE, {first["id"]})
d.write_state(d.STARTED, set())
scripted[:] = [-20, -20, -21]
replies[:] = [("FINISHED", "answered"), ("Wrote the code, the check is clean.\n\nFINISHED", "answered")]
d.main()
k = contract_for(second["id"])
kinds = [e["kind"] for e in k["events"]]
check("a FINISHED over nothing changed is refused on the contract, and it keeps running",
      "finished_refused" in kinds and kinds.index("finished_refused") < kinds.index("turn_started", kinds.index("finished_refused")),
      repr(kinds))
check("a true FINISHED is delivered, not accepted, until a person decides",
      k["state"] == "delivered" and k["decision"] is None and second["id"] in d.finished_missions(), repr(k["state"]))
accepted = as_person({"op": "decide", "contract": k["id"], "verdict": "accepted", "note": ""})
k = contract_for(second["id"])
check("a person's accept makes it accepted, over the events of the revision it applied to",
      accepted["ok"] and k["state"] == "accepted" and k["decision"]["evidence"] == [e["seq"] for e in k["events"][:-1]],
      repr(k["decision"]))
check("a decided contract is never decided again",
      not as_person({"op": "decide", "contract": k["id"], "verdict": "rejected", "note": "changed my mind"})["ok"])
show = c.show_text(k)
with open(os.path.join(tmp, "sample-show.txt"), "w", encoding="utf-8") as f:
    f.write(show)
check("director-contracts show says what was asked, what was done, the evidence and the state, in words",
      all(w in show for w in ("What was asked", "Please build", "What was done", "the Mind said FINISHED; the Director "
                              "refused it", "Evidence", "Last page check: loads clean", "accepted by a person",
                              "legacy, not contract-enforced")), show)

# Stopped mid-mission: by systemctl stop (SIGTERM, a SystemExit through the turn), and by a kill
# that leaves no word at all. Both pick up the same contract, with its events, on the next start.
third = specs[2]
d.write_state(d.DONE, {first["id"], second["id"]})
d.write_state(d.STARTED, set())
os.makedirs(os.path.join(d.PROJECTS, third["folder"]), exist_ok=True)
open(os.path.join(d.PROJECTS, third["folder"], "PLAN.md"), "w").write("- [ ] step 1\n")
real_send = d.send


def sigterm(text):
    raise SystemExit(143)


d.send = sigterm
try:
    d.main()
except SystemExit:
    pass
d.send = real_send
k = contract_for(third["id"])
check("a Director stopped mid-turn leaves its contract blocked with the reason, not running",
      k["state"] == "blocked" and "stopped before the mission ended" in k["reason"], repr((k["state"], k["reason"])))
# Killed outright: the contract is left running, as a SIGKILL would leave it.
c.handle(store, "director", json.dumps({"op": "event", "contract": k["id"], "revision": k["revision"],
                                        "kind": "turn_started", "turn": 1, "prompt": "keep going"}).encode())
held = len(contract_for(third["id"])["events"])
# The service restarts too: a fresh connection to the same file.
store.close()
store = c.connect(os.path.join(tmp, "contracts.db"))
replies[:] = [("working", "card unanswered")]
d.main()
k = contract_for(third["id"])
kinds = [e["kind"] for e in k["events"]]
check("after a restart the same contract continues, its earlier events intact, the cut-off turn said",
      len(k["events"]) > held and kinds[held:held + 3] == ["interrupted", "resumed", "turn_started"]
      and k["state"] == "blocked" and sent[-1].startswith("Please keep going"), repr(kinds))

# A changed brief is a new revision, and the decision of the old one does not carry.
specs_changed = [dict(s) for s in specs]
specs_changed[2]["details"] = specs_changed[2]["details"] + " Add a second level."
real_load = d.load_builds
d.load_builds = lambda: [d.build(s) for s in specs_changed]
replies[:] = [("working", "card unanswered")]
d.main()
d.load_builds = real_load
k = contract_for(third["id"])
check("a changed brief makes a new revision, chained to the one before",
      [(r["revision"], r["parent_revision"]) for r in k["revisions"]] == [(1, None), (2, 1)]
      and "second level" in k["revisions"][1]["intent"]["brief"] and "second level" not in k["revisions"][0]["intent"]["brief"]
      and "revised" in [e["kind"] for e in k["events"]], repr(k["revisions"]))
stale = c.handle(store, "director", json.dumps({"op": "event", "contract": k["id"], "revision": 1, "kind": "stopped",
                                                "turn": 1, "why": "x"}).encode())
check("a report against the old revision is refused", not stale["ok"] and "revision 2" in stale["error"], repr(stale))

# The Director is the process holding this run's key (contracts.py register), because what its
# unit runs reads the Mind's files (security review of #857). It registers before it starts
# anything, sends the key with every report, and nobody else of its account can read it.
order = []
real_become, real_next, real_run_mission = d.become_director, d.next_mission, d.run_mission
d.become_director = lambda: (order.append("register"), "refused: this run of the Director's unit has its key already")[1]
d.next_mission = lambda b: (order.append("next"), None)[1]
d.run_mission = lambda m: order.append("mission")
d.main()
d.become_director = lambda: (order.append("register"), None)[1]
d.main()
d.become_director, d.next_mission, d.run_mission = real_become, real_next, real_run_mission
check("the Director registers before it picks or starts a mission, and starts none when it is refused",
      order == ["register", "register", "next"], repr(order))

import socket
import threading
heard = []
srv_path = os.path.join(tmp, "contracts-fake.sock")
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(srv_path)
srv.listen(4)


def fake_service():
    while True:
        conn, _ = srv.accept()
        with conn:
            raw = b""
            while True:
                chunk = conn.recv(65536)
                if not chunk:
                    break
                raw += chunk
            req = json.loads(raw)
            heard.append(req)
            conn.sendall(json.dumps({"ok": True, "key": "ab" * 32} if req["op"] == "register" else {"ok": True}).encode())


threading.Thread(target=fake_service, daemon=True).start()
real_call = d.contract_call
d.contract_call = real_contract_call
d.CONTRACTS_SOCKET = srv_path
import ctypes
libc = ctypes.CDLL(None)
was_dumpable = libc.prctl(3, 0, 0, 0, 0)
why = real_become_director()
dumpable = libc.prctl(3, 0, 0, 0, 0)
libc.prctl(d.PR_SET_DUMPABLE, was_dumpable, 0, 0, 0)
d.contract_call({"op": "event", "contract": "c-1"})
d.contract_call = real_call
check("become_director makes the Director non-dumpable, then asks for the key with nothing else",
      why is None and dumpable == 0 and heard[:1] == [{"op": "register"}], repr((why, dumpable, heard)))
check("every report after it carries the key, which is in no file, argument or environment",
      heard[1:] == [{"op": "event", "contract": "c-1", "key": "ab" * 32}]
      and "ab" * 32 not in json.dumps(dict(os.environ)), repr(heard))
d.DIRECTOR_KEY[:] = []

# yos as the Director runs it does not read the Mind's user site: a .pth planted under HOME.
import subprocess
mindhome = os.path.join(tmp, "mindhome")
py = d.YOS_RUN[0] if os.path.exists(d.YOS_RUN[0]) else sys.executable
site_dir = subprocess.run([py, "-c", "import site; print(site.getusersitepackages())"], env={"HOME": mindhome},
                          capture_output=True, text=True).stdout.strip()
os.makedirs(site_dir)
marker = os.path.join(mindhome, "pth-ran")
with open(os.path.join(site_dir, "mind.pth"), "w") as f:
    f.write(f"import os; open({marker!r}, 'w').close()\n")
fake_yos = os.path.join(tmp, "yos")
with open(fake_yos, "w") as f:
    f.write("print('shell')\nprint('{')\nprint('\"thinking\": false')\nprint('}')\n")
real_yos_run, real_home = d.YOS_RUN, os.environ.get("HOME")
os.environ["HOME"] = mindhome
os.environ.pop("PYTHONNOUSERSITE", None)
d.YOS_RUN = (py, *real_yos_run[1:-1], fake_yos)
seen = real_shell_state()
ran_isolated = os.path.exists(marker)
subprocess.run([py, fake_yos], capture_output=True)
ran_plain = os.path.exists(marker)
d.YOS_RUN = real_yos_run
os.environ["HOME"] = real_home or "/"
check("yos runs with -I, so a .pth the Mind left under its HOME does not run in the Director's unit "
      "(it does in a plain python3)",
      real_yos_run[:2] == ("/usr/bin/python3", "-I") and seen == {"thinking": False} and not ran_isolated and ran_plain,
      repr((real_yos_run, seen, ran_isolated, ran_plain)))
with open(os.path.join(HERE, "check-page"), encoding="utf-8") as f:
    page_script = f.read()
check("check-page runs chromium with HOME its throwaway profile, never the Mind's (nssdb, fontconfig)",
      'export HOME="$profile"' in page_script
      and page_script.index('export HOME="$profile"') < page_script.index("chromium --headless"))

sys.exit(1 if failed else 0)
