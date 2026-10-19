#!/usr/bin/env python3
"""The contracts store's own rules (contracts.py), with the service run for real on a scratch
socket where it can be.

Holds what the design forbids breaking: only a person accepts; nothing the Director, or the
Mind's account, can send reaches `accepted`; a malformed report is refused whole and leaves the
store as it was; revisions and events are never rewritten. The Director's side (a FINISHED is
delivered, a restart keeps the contract, a changed brief is a revision) is in director-selftest.py.
Run as root it also starts the service and tries it as another account (nobody), through the
kernel's own peer credentials. Exit 0 when every check holds.
"""
import importlib.util
import json
import os
import pwd
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("contracts", os.path.join(HERE, "contracts.py"))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)

tmp = tempfile.mkdtemp()
failed = 0


def check(name, ok, said=""):
    global failed
    print(("ok    " if ok else "FAIL  ") + name + ("" if ok else ": " + str(said)[:300]))
    failed += not ok


def ask(db, actor, req):
    return c.handle(db, actor, req if isinstance(req, bytes) else json.dumps(req).encode())


def open_req(**over):
    req = {"op": "open", "mission": "starfall", "title": "Starfall", "brief": "Please build Starfall.",
           "folder": "starfall", "page": "~/Projects/starfall/index.html", "resume": False}
    req.update(over)
    return req


def counts(db):
    return [db.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0] for t in ("contracts", "revisions", "events")] \
        + [db.execute("SELECT state FROM contracts ORDER BY rowid").fetchall()]


db = c.connect(os.path.join(tmp, "store.db"))
opened = ask(db, "director", open_req())
cid = opened["contract"]
check("the Director opens a contract, ready at revision 1",
      opened["ok"] and opened["revision"] == 1 and c.contract_view(db, cid)["state"] == "ready", repr(opened))
started = ask(db, "director", {"op": "event", "contract": cid, "revision": 1, "kind": "turn_started", "turn": 1,
                               "prompt": "Please build Starfall."})
check("a turn starting makes it running", started == {"ok": True, "contract": cid, "state": "running"}, repr(started))

H = "a" * 64
good = {"op": "event", "contract": cid, "revision": 1, "kind": "page_check", "turn": 1, "result": "loads clean",
        "output": ""}
malformed = {
    "not JSON": b"{nope",
    "not UTF-8": b"\xff\xfe",
    "not an object": b"[1, 2]",
    "nested deep enough to raise": b"[" * 100000,
    "over the size limit": json.dumps(dict(good, output="x" * (c.REQUEST_MAX_BYTES + 1))).encode(),
    "an unknown op": {"op": "accept", "contract": cid},
    "an unknown kind": dict(good, kind="accepted"),
    "a kind that is a list": dict(good, kind=["page_check"]),
    "a field too many": dict(good, state="accepted"),
    "a field missing": {k: v for k, v in good.items() if k != "output"},
    "a turn that is text": dict(good, turn="1"),
    "a turn that is true": dict(good, turn=True),
    "a revision that is true": dict(good, revision=True),
    "a stale revision": dict(good, revision=0),
    "a result not on the list": dict(good, result="passed, trust me"),
    "a hash that is not one": {"op": "event", "contract": cid, "revision": 1, "kind": "file_changed", "turn": 1,
                               "before": H, "after": "../../etc", "files": []},
    "too many files": {"op": "event", "contract": cid, "revision": 1, "kind": "file_changed", "turn": 1,
                       "before": H, "after": H, "files": ["a"] * (c.FILES_MAX + 1)},
    "text over its limit": dict(good, output="x" * (c.TEXT_MAX + 1)),
    "a contract id that is not one": dict(good, contract="c-1' OR 1=1"),
    "an unknown contract": dict(good, contract="c-999"),
    "open with a field too many": open_req(grant={"rule": "everything"}),
    "open with a mission id that climbs": open_req(mission="../starfall"),
    "open with resume as text": open_req(resume="yes"),
}
before = counts(db)
took = {name: ask(db, "director", req) for name, req in malformed.items()}
check("every malformed report is refused whole", not [n for n, r in took.items() if r.get("ok")],
      [n for n, r in took.items() if r.get("ok")])
check("and the store is exactly as it was", counts(db) == before, repr((before, counts(db))))
check("a refusal says why in words", all(isinstance(r.get("error"), str) and r["error"] for r in took.values()),
      repr(took))

hostile = ask(db, "director", dict(good, output="\x1b[2J\u202egnp.exe\u200b\nfake line"))
shown = c.event_words(c.contract_view(db, cid)["events"][-1])
check("terminal escapes, bidi and zero-width characters are taken out before they are stored or shown",
      hostile["ok"] and "\x1b" not in shown and "\u202e" not in shown and "\u200b" not in shown and "\n" not in shown,
      repr(shown))

# Who may ask what: the kernel's answer, never the caller's.
mind = 1000
check("a person is root in any cgroup but a yantrik-live-* service's: a login session, the guest agent, another daemon",
      all(c.actor_for(0, where, mind) == "person" for where in (
          "/user.slice/user-0.slice/session-3.scope", "/system.slice/qemu-guest-agent.service",
          "/system.slice/yantrik-egress.service", "/system.slice/yantrik-live-x.serviceX", "/")))
check("root in a live unit, or under one, is not a person",
      all(c.actor_for(0, where, mind) is None for where in (
          "/system.slice/yantrik-live-showcase.service", "/system.slice/yantrik-live-director.service",
          "/system.slice/yantrik-live-showcase.service/sub", c.DIRECTOR_CGROUP)))
check("the Mind's account in the Director's unit may only try its key; anywhere else, and anyone else, are nobody",
      c.actor_for(mind, c.DIRECTOR_CGROUP, mind) == "director unit"
      and c.actor_for(mind, "/user.slice/user-1000.slice/session-2.scope", mind) is None
      and c.actor_for(mind, c.DIRECTOR_CGROUP + "/sub", mind) is None
      and c.actor_for(0, None, mind) is None and c.actor_for(1001, c.DIRECTOR_CGROUP, mind) is None)
before = counts(db)
nobody = [ask(db, None, r) for r in (open_req(), good, {"op": "list"},
                                      {"op": "decide", "contract": cid, "verdict": "accepted", "note": ""})]
director = [ask(db, "director", r) for r in ({"op": "list"}, {"op": "show", "contract": cid},
                                              {"op": "decide", "contract": cid, "verdict": "accepted", "note": ""})]
person = [ask(db, "person", r) for r in (open_req(), good)]
check("a caller that is nobody may ask nothing, the Director may not read or decide, a person may not report",
      not any(r["ok"] for r in nobody + director + person) and counts(db) == before, repr(nobody + director + person))
check("a person cannot accept what was never delivered",
      not ask(db, "person", {"op": "decide", "contract": cid, "verdict": "accepted", "note": ""})["ok"])
ask(db, "director", {"op": "event", "contract": cid, "revision": 1, "kind": "finished_said", "turn": 1,
                     "mind_said": "FINISHED", "folder": H, "page": "loads clean"})
check("a reason is needed to reject",
      not ask(db, "person", {"op": "decide", "contract": cid, "verdict": "rejected", "note": "  "})["ok"])

# A changed brief is proposed again by the Director and activated again by the standing policy,
# each said in the log by whoever did it (review of #857: it went straight back to ready).
rdb = c.connect(os.path.join(tmp, "revised.db"))
r1 = ask(rdb, "director", open_req())
r2 = ask(rdb, "director", open_req(brief="Please build Starfall, with a second level.", resume=True))
tail = [(e["actor"], e["kind"], e["state"]) for e in c.contract_view(rdb, r1["contract"])["events"]][-2:]
check("a new revision is proposed by the Director, then activated by the standing policy, never ready by itself",
      r2.get("revision") == 2 and tail == [("director", "revised", "proposed"), ("standing policy", "activated", "ready")],
      repr((r2, tail)))

# Credentials in a command are stored and shown as [redacted] (review of #857).
secrets_in = ['curl -H "Authorization: Bearer abcdef0123456789" https://user:hunter2@example.com/x?api-key=K3Y&b=1',
              "export OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwx; mysql --password pa55word",
              "git push https://ghp_abcdefghijklmnopqrstuvwxyz0123@github.com/x token=t0k3n"]
for i, cmd in enumerate(secrets_in):
    ask(rdb, "director", {"op": "event", "contract": r1["contract"], "revision": 2, "kind": "turn_started", "turn": i + 1,
                          "prompt": "go"}) if i == 0 else None
    ask(rdb, "director", {"op": "event", "contract": r1["contract"], "revision": 2, "kind": "command_run", "turn": 1,
                          "command": cmd})
stored = json.dumps(c.contract_view(rdb, r1["contract"])["events"]) + c.show_text(c.show_view(rdb, r1["contract"]))
leaked = [w for w in ("abcdef0123456789", "hunter2", "K3Y", "sk-abcdef", "pa55word", "ghp_abc", "t0k3n") if w in stored]
check("credentials in a command (Authorization, user:pw@, api-key=, sk-/ghp_ keys, --password, token=) are stored "
      "and shown as [redacted]", not leaked and stored.count("[redacted]") >= 7 and "curl -H" in stored, leaked)
check("words that only look like credentials are kept",
      c.redact("the password field is empty; MAX_TOKENS=5; tokens are fine") ==
      "the password field is empty; MAX_TOKENS=5; tokens are fine")

# Any contract can be shown: a reply never past what the person's command reads, however many
# events of whatever size (review of #857: 5,000 events of 4,000 characters made `show` fail).
big = c.connect(os.path.join(tmp, "big.db"))
b1 = ask(big, "director", open_req())["contract"]
ask(big, "director", {"op": "event", "contract": b1, "revision": 1, "kind": "turn_started", "turn": 1, "prompt": "go"})
widest = ["\U0001d11e" * c.NAME_MAX] * c.FILES_MAX
n_big = 2 * c.SHOW_EVENTS + 10
for i in range(n_big):
    # As UTF-8, as it fits a request (escaped, 50 names of 200 such characters would not).
    assert c.handle(big, "director", json.dumps({"op": "event", "contract": b1, "revision": 1, "kind": "file_changed",
                                                 "turn": 1, "before": H, "after": "%064x" % i, "files": widest},
                                                ensure_ascii=False).encode())["ok"]
whole = len(json.dumps(c.contract_view(big, b1), ensure_ascii=False).encode())
shown_big = ask(big, "person", {"op": "show", "contract": b1})
size = len(json.dumps(shown_big, ensure_ascii=False).encode())
text_big = c.show_text(shown_big["contract"])
check("show of a contract past REPLY_MAX_BYTES in events sends the last SHOW_EVENTS, with the evidence over all",
      whole > c.REPLY_MAX_BYTES and size < c.REPLY_MAX_BYTES and len(shown_big["contract"]["events"]) == c.SHOW_EVENTS
      and f"in {n_big} turns" in text_big and f"({n_big + 3} events; the last {c.SHOW_EVENTS} are shown)" in text_big
      and "%064x" % (n_big - 1) in json.dumps(shown_big),
      repr((whole, size, text_big[-600:])))
check("and the bound holds at any size of event: every reply at its largest fits (EVENT_BYTES_MAX)",
      size < (c.SHOW_EVENTS + 3) * c.EVENT_BYTES_MAX and c.EVENT_BYTES_MAX >= 4 * c.FILES_MAX * c.NAME_MAX)

# An accept that is recorded is said to be, even when showing it again then fails.
real_call, real_euid = c.call, c.os.geteuid
replies_cli = [{"ok": True, "contract": "c-9", "state": "accepted"}]
c.call = lambda req, path=None: replies_cli.pop(0) if replies_cli else (_ for _ in ()).throw(OSError("gone"))
c.os.geteuid = lambda: 0
import contextlib
import io
out_cli, err_cli = io.StringIO(), io.StringIO()
with contextlib.redirect_stdout(out_cli), contextlib.redirect_stderr(err_cli):
    code_cli = c.cli(["accept", "c-9"])
c.call, c.os.geteuid = real_call, real_euid
check("director-contracts accept says the decision is recorded when the show after it fails, and exits 0",
      code_cli == 0 and "c-9 is now accepted" in out_cli.getvalue() and "decision is recorded" in err_cli.getvalue(),
      repr((code_cli, out_cli.getvalue(), err_cli.getvalue())))

# The Director is the process holding this run's key, not anything in its unit (review of #857).
# A scratch "unit": its directory's inode is the run, its cgroup.procs who is in it.
unit = os.path.join(tmp, "unit-run-1")
os.mkdir(unit)


def in_unit(*pids, where=None):
    with open(os.path.join(where or unit, "cgroup.procs"), "w") as f:
        f.write("".join(f"{p}\n" for p in pids))


real_unit_dir, c.DIRECTOR_UNIT_DIR = c.DIRECTOR_UNIT_DIR, unit
kdb = c.connect(os.path.join(tmp, "keys.db"))


def as_unit(req, pid=4242):
    return c.handle(kdb, "director unit", json.dumps(req).encode(), peer_pid=pid)


in_unit(4242, 4243)
crowded = as_unit({"op": "register"})
in_unit(4242)
reg = as_unit({"op": "register"})
again = as_unit({"op": "register"})
in_unit(4243)
other = as_unit({"op": "register"}, pid=4243)
key = reg.get("key", "")
check("a run's key is given once, only to the one process in the unit",
      not crowded["ok"] and reg["ok"] and c.KEY.match(key) and not again["ok"] and not other["ok"]
      and "already" in again["error"], repr((crowded, reg, again, other)))
tries = {"no key": as_unit(open_req()), "an empty key": as_unit(dict(open_req(), key="")),
         "a guessed key": as_unit(dict(open_req(), key="0" * 64)), "a key that is a list": as_unit(dict(open_req(), key=[key])),
         "register with a field": as_unit({"op": "register", "key": key})}
check("in the Director's unit, without this run's key, nothing is taken", not any(r["ok"] for r in tries.values())
      and kdb.execute("SELECT COUNT(*) FROM contracts").fetchone() == (0,), repr(tries))
mine = as_unit(dict(open_req(), key=key))
check("with it, the Director's reports are taken", mine["ok"], repr(mine))
kdb.execute("SELECT 1").fetchall()
try:
    kdb.execute("UPDATE director_runs SET key_hash='x'")
    rekeyed = True
except sqlite3.DatabaseError:
    rekeyed = False
unit2 = os.path.join(tmp, "unit-run-2")
os.mkdir(unit2)
in_unit(5000, where=unit2)
c.DIRECTOR_UNIT_DIR = unit2
stale_key = as_unit(dict(open_req(), key=key))
check("a key is good for its own run only, and a run's key is never changed", not stale_key["ok"] and not rekeyed,
      repr(stale_key))
c.DIRECTOR_UNIT_DIR = real_unit_dir

# A contract at its event cap still takes a person's decision.
capped = c.connect(os.path.join(tmp, "capped.db"))
k = ask(capped, "director", open_req())["contract"]
ask(capped, "director", {"op": "event", "contract": k, "revision": 1, "kind": "turn_started", "turn": 1, "prompt": "go"})
ask(capped, "director", {"op": "event", "contract": k, "revision": 1, "kind": "finished_said", "turn": 1,
                         "mind_said": "FINISHED", "folder": H, "page": "loads clean"})
real_max, c.EVENTS_MAX = c.EVENTS_MAX, 4
decided = ask(capped, "person", {"op": "decide", "contract": k, "verdict": "accepted", "note": ""})
k2 = ask(capped, "director", open_req(mission="orrery"))["contract"]
ask(capped, "director", {"op": "event", "contract": k2, "revision": 1, "kind": "turn_started", "turn": 1, "prompt": "go"})
ask(capped, "director", dict(good, contract=k2))
over = ask(capped, "director", dict(good, contract=k2))
c.EVENTS_MAX = real_max
check("past its event cap a contract takes no more reports, but still a person's decision",
      not over["ok"] and "most it keeps" in over["error"] and decided["ok"], repr((over, decided)))

# Below the service: the store's own triggers hold even against SQL written straight at it.
refused = []
for sql in ("UPDATE events SET kind='accepted' WHERE seq=1", "DELETE FROM events", "UPDATE revisions SET intent='{}'",
            "DELETE FROM revisions", f"UPDATE contracts SET state='accepted' WHERE id='{cid}'", "DELETE FROM contracts"):
    try:
        db.execute(sql)
    except sqlite3.DatabaseError:
        refused.append(sql)
check("events, revisions and contracts are never rewritten or deleted, and accepted needs a decision, even in SQL",
      len(refused) == 6,
      refused)
ask(db, "person", {"op": "decide", "contract": cid, "verdict": "accepted", "note": "plays well"})
try:
    db.execute(f"UPDATE contracts SET state='running' WHERE id='{cid}'")
    reopened = True
except sqlite3.DatabaseError:
    reopened = False
check("an accepted contract cannot be moved again", not reopened)

# The person's command and the service, for real: a scratch socket and store, the kernel's peer
# credentials. As root only (it needs a second account to try).
if os.geteuid() != 0:
    print("skip  the service as another account: needs root to start it and to run as nobody")
else:
    c.SOCKET = os.path.join(tmp, "contracts.sock")
    c.DB = os.path.join(tmp, "store-dir", "contracts.db")
    os.mkdir(os.path.dirname(c.DB), 0o700)
    os.chmod(tmp, 0o755)
    other = pwd.getpwnam("nobody")
    # A copy the other account can read wherever the checkout is (a CI home is often 0750).
    import shutil
    shutil.copy(os.path.join(HERE, "contracts.py"), os.path.join(tmp, "contracts.py"))
    os.chmod(os.path.join(tmp, "contracts.py"), 0o644)
    c.MIND_ACCOUNT = "nobody"
    threading.Thread(target=c.serve, daemon=True).start()
    for _ in range(50):
        if os.path.exists(c.SOCKET):
            break
        time.sleep(0.1)
    check("the service starts and listens", os.path.exists(c.SOCKET))
    # The store made by the service: a delivered contract to try to accept.
    sdb = sqlite3.connect(c.DB)
    ask(sdb, "director", open_req())
    ask(sdb, "director", {"op": "event", "contract": "c-1", "revision": 1, "kind": "turn_started", "turn": 1,
                          "prompt": "go"})
    ask(sdb, "director", {"op": "event", "contract": "c-1", "revision": 1, "kind": "finished_said", "turn": 1,
                          "mind_said": "FINISHED", "folder": H, "page": "loads clean"})
    sdb.close()
    probe = subprocess.run([sys.executable, "-I", "-c", "import sys; sys.path.insert(0, %r); import contracts; "
                            "sys.exit(contracts.probe(%r, %r))" % (tmp, c.SOCKET, c.DB)],
                           user=other.pw_uid, group=other.pw_gid, capture_output=True, text=True, timeout=30, cwd="/")
    check("the Mind's account cannot accept, report, or open the store: every try refused",
          probe.returncode == 0 and probe.stdout.count("refused") == 3 and "ACCEPTED" not in probe.stdout
          and "REPORTED" not in probe.stdout and "OPENED" not in probe.stdout,
          probe.stdout + probe.stderr)
    after = sqlite3.connect(c.DB)
    check("and the delivered contract is still delivered, and no contract was opened by it",
          after.execute("SELECT state FROM contracts WHERE id='c-1'").fetchone() == ("delivered",)
          and after.execute("SELECT COUNT(*) FROM contracts WHERE mission='probe'").fetchone() == (0,))
    after.close()
    cli_other = subprocess.run([sys.executable, "-I", os.path.join(tmp, "contracts.py"), "cli", "accept", "c-1"],
                               user=other.pw_uid, group=other.pw_gid, capture_output=True, text=True, timeout=30, cwd="/")
    check("director-contracts refuses to run as any account but root", cli_other.returncode == 1
          and "root's" in cli_other.stderr, cli_other.stderr)

    def run_cli(*args):
        import contextlib
        import io
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = c.cli(list(args), c.SOCKET)
        return code, out.getvalue(), err.getvalue()

    listed = run_cli("list")
    check("director-contracts list names each contract, its state, and what waits for a person",
          listed[0] == 0 and "c-1" in listed[1] and "delivered" in listed[1] and "waiting for a person" in listed[1],
          repr(listed))
    shown = run_cli("show", "c-1")
    check("director-contracts show tells a person how to decide a delivered contract",
          shown[0] == 0 and "Not accepted: a person decides" in shown[1] and "director-contracts accept c-1" in shown[1],
          repr(shown))
    no_reason = run_cli("reject", "c-1")
    accepted = run_cli("accept", "c-1", "plays", "well")
    check("director-contracts accept, as root, accepts; reject without a reason is not taken",
          no_reason[0] == 2 and accepted[0] == 0 and "c-1 is now accepted" in accepted[1]
          and "accepted by a person" in accepted[1] and "plays well" in accepted[1], repr((no_reason, accepted)))
    with open(os.path.join(tmp, "sample-show.txt"), "w", encoding="utf-8") as f:
        f.write(accepted[1])

    # One slow caller holds one worker, for REQUEST_SECS at most; the others are answered.
    import socket
    slow = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    slow.connect(c.SOCKET)
    t0 = time.monotonic()
    quick = run_cli("list")
    quick_secs = time.monotonic() - t0
    slow.settimeout(c.REQUEST_SECS + 5)
    dropped, t0 = None, time.monotonic()
    try:
        for _ in range(2 * (c.REQUEST_SECS + 5)):
            slow.sendall(b" ")
            time.sleep(0.5)
    except OSError as why:
        dropped = why
    held = time.monotonic() - t0
    slow.close()
    check("a caller sending a byte at a time is cut off at the request's deadline, and others are answered meanwhile",
          quick[0] == 0 and quick_secs < 2 and dropped is not None and held < c.REQUEST_SECS + 2,
          repr((quick[0], quick_secs, dropped, held)))

    # Past its workers a caller is told the service is busy at once, never queued behind the slow.
    hold = []
    for _ in range(c.WORKERS["person"]):
        hold.append(socket.socket(socket.AF_UNIX, socket.SOCK_STREAM))
        hold[-1].connect(c.SOCKET)
    time.sleep(0.3)
    t0 = time.monotonic()
    busy = run_cli("list")
    busy_secs = time.monotonic() - t0
    for h in hold:
        h.close()
    time.sleep(0.3)
    check("with every person's worker held, the next person is told busy at once; workers come back",
          busy[0] == 1 and "busy" in busy[2] and busy_secs < 2 and run_cli("list")[0] == 0, repr((busy, busy_secs)))

    # The forging path of the review of #857, for real: a Director, as the Mind's account in the
    # Director's unit, starts a `yos` without -I under the Mind's HOME, where the Mind left a .pth.
    # The .pth runs, in the unit, as the account, and tries to report in the Director's name.
    # Here "the unit" is this test's own cgroup and a scratch cgroup.procs.
    c.DIRECTOR_CGROUP = c.cgroup_of(os.getpid())
    c.DIRECTOR_UNIT_DIR = os.path.join(tmp, "unit-live")
    os.mkdir(c.DIRECTOR_UNIT_DIR, 0o755)
    mindhome = os.path.join(tmp, "mindhome")
    site_dir = subprocess.run([sys.executable, "-c", "import site; print(site.getusersitepackages())"],
                              env={"HOME": mindhome}, capture_output=True, text=True).stdout.strip()
    os.makedirs(site_dir)
    with open(os.path.join(site_dir, "forge.pth"), "w") as f:
        f.write("import forge_mind\n")
    with open(os.path.join(site_dir, "forge_mind.py"), "w") as f:
        f.write(f"""import json, os, sys
sys.path.insert(0, {tmp!r})
import contracts
sock, cid, rev = os.environ["FORGE"].split(" ")
rev = int(rev)
done = {{"page": "loads clean", "folder": "a" * 64, "mind_said": "FINISHED", "turn": 1}}
forged = {{}}
for name, req in (("register", {{"op": "register"}}),
                  ("delivered, no key", dict(done, op="event", contract=cid, revision=rev, kind="finished_said")),
                  ("delivered, a guessed key", dict(done, op="event", contract=cid, revision=rev, kind="finished_said",
                                                    key="f" * 64)),
                  ("fake evidence", {{"op": "event", "contract": cid, "revision": rev, "kind": "page_check", "turn": 1,
                                     "result": "loads clean", "output": "", "key": None}}),
                  ("start over", {{"op": "open", "mission": "starfall", "title": "x", "brief": "x", "folder": "x",
                                  "page": "", "resume": False}})):
    try:
        forged[name] = contracts.call(req, sock)
    except Exception as why:
        forged[name] = {{"ok": False, "error": repr(why)}}
# The key is in the Director's memory: try to read it there, as the account may of its own processes.
for name in ("mem", "environ", "fd"):
    try:
        path = f"/proc/{{os.getppid()}}/{{name}}"
        os.listdir(path) if name == "fd" else open(path, "rb").read(1)
        forged["read the Director's " + name] = {{"ok": True}}
    except OSError as why:
        forged["read the Director's " + name] = {{"ok": False, "error": type(why).__name__}}
open(os.path.join(os.environ["HOME"], "forged.json"), "w").write(json.dumps(forged))
""")
    with open(os.path.join(tmp, "yos_fake.py"), "w") as f:
        f.write("print('{}')\n")
    with open(os.path.join(tmp, "director_fake.py"), "w") as f:
        f.write(f"""import json, subprocess, sys
sys.path.insert(0, {tmp!r})
import contracts
import ctypes
sock = sys.argv[1]
# As director.py become_director does, unless this test is showing what that step is for.
if sys.argv[2:] != ["dumpable"]:
    assert ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) == 0
sys.stdin.readline()
out = {{"register": contracts.call({{"op": "register"}}, sock)}}
key = out["register"].get("key")
out["open"] = contracts.call({{"op": "open", "mission": "starfall", "title": "Starfall", "brief": "Please build it.",
                               "folder": "starfall", "page": "", "resume": False, "key": key}}, sock)
cid, rev = out["open"]["contract"], out["open"]["revision"]
out["turn"] = contracts.call({{"op": "event", "contract": cid, "revision": rev, "kind": "turn_started", "turn": 1,
                               "prompt": "go", "key": key}}, sock)
subprocess.run([sys.executable, {os.path.join(tmp, "yos_fake.py")!r}], timeout=60, capture_output=True,
               env={{"HOME": {mindhome!r}, "PATH": "/usr/bin:/bin", "FORGE": f"{{sock}} {{cid}} {{rev}}"}})
out["page"] = contracts.call({{"op": "event", "contract": cid, "revision": rev, "kind": "page_check", "turn": 1,
                               "result": "throws", "output": "TypeError", "key": key}}, sock)
print(json.dumps(out))
""")
    # The service thread set this process's umask to 077: what the other account reads is opened up.
    for top, dirs, names in os.walk(mindhome):
        for n in dirs + names:
            os.chmod(os.path.join(top, n), 0o755 if n in dirs else 0o644)
    for n in ("yos_fake.py", "director_fake.py"):
        os.chmod(os.path.join(tmp, n), 0o644)
    os.chown(mindhome, other.pw_uid, other.pw_gid)
    def run_director(*args):
        """One run of the stand-in Director in a new run of the stand-in unit: what it said, what
        the .pth tried, and the contract it opened (its events and state)."""
        c.DIRECTOR_UNIT_DIR = tempfile.mkdtemp(dir=tmp)
        forged_path = os.path.join(mindhome, "forged.json")
        if os.path.exists(forged_path):
            os.unlink(forged_path)
        director = subprocess.Popen([sys.executable, "-I", os.path.join(tmp, "director_fake.py"), c.SOCKET, *args],
                                    user=other.pw_uid, group=other.pw_gid, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, text=True, cwd="/")
        in_unit(director.pid, where=c.DIRECTOR_UNIT_DIR)
        out, err = director.communicate("go\n", timeout=120)
        said = json.loads(out or "{}")
        forged = json.load(open(forged_path)) if os.path.exists(forged_path) else None
        live = sqlite3.connect(c.DB)
        cid_live = (said.get("open") or {}).get("contract")
        log_live = live.execute("SELECT actor, kind FROM events WHERE contract=? ORDER BY seq", (cid_live,)).fetchall()
        state_live = live.execute("SELECT state FROM contracts WHERE id=?", (cid_live,)).fetchone()
        live.close()
        return out, err, said, forged, log_live, state_live

    out, err, said, forged, log_live, state_live = run_director()
    check("the Director, as the Mind's account in its unit, registers and reports with its run's key",
          all((said.get(k) or {}).get("ok") for k in ("register", "open", "turn", "page")), out + err)
    check("a .pth the Mind left runs in a yos started without -I, inside the Director's unit (the path is real)",
          forged is not None and len(forged) == 8, repr(forged) + err)
    check("and nothing it sends is taken: no key, no second key, no delivered, no evidence, no starting over",
          forged is not None and not any(r.get("ok") for r in forged.values())
          and state_live == ("running",)
          and log_live == [("director", "proposed"), ("standing policy", "activated"), ("director", "turn_started"),
                           ("director", "page_check")], repr((forged, state_live, log_live)))
    _o, _e, _s, open_forged, _l, _st = run_director("dumpable")
    check("the Director being non-dumpable is what keeps its memory and environment from the .pth: a dumpable one's "
          "environment is read",
          (open_forged or {}).get("read the Director's environ", {}).get("ok") is True
          and not any(v.get("ok") for k, v in forged.items() if k.startswith("read")), repr((open_forged, forged)))
    check("the store made by the service is the service's alone (0600 under a 0700 directory)",
          os.stat(c.DB).st_mode & 0o077 == 0, oct(os.stat(c.DB).st_mode))

units = {}
for name in ("yantrik-live-contracts.service", "yantrik-live-director.service"):
    with open(os.path.join(HERE, name), encoding="utf-8") as f:
        units[name] = [line.strip() for line in f if not line.lstrip().startswith("#")]
svc = units["yantrik-live-contracts.service"]
check("the contracts service runs as its own account, its store 0700, no network, nothing else writable",
      all(x in svc for x in ("User=yantrik-contracts", "StateDirectory=yantrik-contracts", "StateDirectoryMode=0700",
                             "RuntimeDirectory=yantrik-contracts", "PrivateNetwork=yes", "ProtectSystem=strict",
                             "NoNewPrivileges=yes", "CapabilityBoundingSet=", "RestrictAddressFamilies=AF_UNIX"))
      and any(x.startswith("ExecStart=/usr/bin/python3 -I ") and x.endswith("contracts.py serve") for x in svc), svc)
check("the Director runs Python isolated, and no Python it starts reads the Mind's user site (PYTHONNOUSERSITE); "
      "the key, not the unit, is what makes the Director (checked above)",
      any(x.startswith("ExecStart=/usr/bin/python3 -I ") for x in units["yantrik-live-director.service"])
      and "Environment=PYTHONNOUSERSITE=1" in units["yantrik-live-director.service"])
check("setup fails when ptrace_scope is 0, unless ALLOW_PTRACE_SCOPE_0=1 is set on purpose",
      "kernel.yama.ptrace_scope is not 1 or more" in open(os.path.join(HERE, "setup-director.sh")).read()
      and "ALLOW_PTRACE_SCOPE_0" in open(os.path.join(HERE, "setup-director.sh")).read())
with open(os.path.join(HERE, "setup-director.sh"), encoding="utf-8") as f:
    setup = f.read()
check("setup-director.sh installs the store's module, the service, its account and the person's command",
      all(x in setup for x in ("contracts.py", "yantrik-live-contracts.service", "useradd", "yantrik-contracts",
                               "/usr/local/sbin/director-contracts")), "")

sys.exit(1 if failed else 0)
