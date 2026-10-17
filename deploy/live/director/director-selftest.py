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
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("director", os.path.join(HERE, "director.py"))
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)

tmp = tempfile.mkdtemp()
d.STATE_DIR = tmp
for k in ("LOG", "DONE", "STARTED", "POLISHED", "POLISH_DUE", "LAST", "SHOWING"):
    setattr(d, k, os.path.join(tmp, os.path.basename(getattr(d, k))))
d.PROJECTS = os.path.join(tmp, "Projects")
d.SHOWCASE_DIR = os.path.join(tmp, "showcase")
# Builds in order first; interleaving with polish rounds has its own checks further down.
d.POLISH_PER_BUILD = 0
d.BETWEEN_TURNS_SECS = 0
d.time.sleep = lambda s: None
d.show_mind_view = lambda: None
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
publisher_unit = unit_lines("yantrik-live-showcase.service")
browser_unit = unit_lines("yantrik-live-showcase-browser.service")
windows_unit = unit_lines("yantrik-live-windows.service")


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
      and setting(browser_unit, "ExecStartPre") == "+/usr/bin/python3 -I /opt/yantrik-live/director/director.py wayland-grant")
check("the window loop is the desktop's account and runs no browser",
      "User=yantrik" in windows_unit and "IPAddressDeny=any" in windows_unit
      and setting(windows_unit, "ExecStart") == "/usr/bin/python3 /opt/yantrik-live/director/director.py windows"
      and "showcase-browser" not in setting(windows_unit, "ExecStart"))
setup = open(os.path.join(HERE, "setup-director.sh"), encoding="utf-8").read()
check("setup makes the showcase root's, the browser's account, and the root-only rotation, and installs all three"
      " units", "install -d -o root -g root -m 755 /opt/yantrik-live/showcase" in setup
      and "-o yantrik -g yantrik -m 755 /opt/yantrik-live/showcase" not in setup
      and "useradd --system" in setup and "yantrik-showcase" in setup and "/etc/yantrik-live/showcase-rotation" in setup
      and all(u in setup for u in ("yantrik-live-showcase.service", "yantrik-live-showcase-browser.service",
                                   "yantrik-live-windows.service")))

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

sys.exit(1 if failed else 0)
