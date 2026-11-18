"""The Director's stand-ins for director-selftest.py: director.py and contracts.py loaded from this
folder, with the desktop, the Mind and the page check replaced by lists a test fills, every state
file in a scratch folder, and check(). Split out of director-selftest.py, which was over 600 lines
(#870); a test that rebinds `store` here rebinds it here, where contract_call reads it.
"""
import importlib.util
import json
import os
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
