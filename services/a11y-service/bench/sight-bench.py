#!/usr/bin/env python3
"""Time the screen model against a window that says when it changed.

usage: sight-bench.py <a11y.sock> <sight-demo-app.py> <service pid>
Prints one JSON report on stdout. Reads no text out of any window but the demo's own.
"""
import json
import os
import socket
import subprocess
import sys
import threading
import time

SOCK, APP, PID = sys.argv[1], sys.argv[2], int(sys.argv[3])
TICK = os.sysconf("SC_CLK_TCK")


def now_ms():
    return int(time.time() * 1000)


class Rpc:
    """One connection, one JSON object per line."""

    def __init__(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(SOCK)
        self.file = self.sock.makefile("rwb")
        self.n = 0

    def call(self, method, params=None):
        self.n += 1
        line = json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}})
        started = time.perf_counter()
        self.file.write(line.encode() + b"\n")
        self.file.flush()
        reply = self.file.readline()
        took_ms = (time.perf_counter() - started) * 1000
        answer = json.loads(reply)
        if "error" in answer and answer["error"]:
            raise RuntimeError(f"{method}: {answer['error']}")
        return answer.get("result"), took_ms, len(reply)


def cpu_ms(pid):
    with open(f"/proc/{pid}/stat") as stat:
        fields = stat.read().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) * 1000 // TICK


def threads_cpu_ms(pid):
    """CPU per thread name, so a busy run says which thread was busy."""
    by_name = {}
    for task in os.listdir(f"/proc/{pid}/task"):
        try:
            with open(f"/proc/{pid}/task/{task}/stat") as stat:
                name, rest = stat.read().split("(", 1)[1].rsplit(")", 1)
        except OSError:
            continue
        fields = rest.split()
        by_name[name] = by_name.get(name, 0) + (int(fields[11]) + int(fields[12])) * 1000 // TICK
    return by_name


def spread(values):
    values = sorted(values)
    pick = lambda q: round(values[min(len(values) - 1, int(len(values) * q))], 2)
    return {"n": len(values), "p50": pick(0.5), "p95": pick(0.95), "max": round(values[-1], 2)}


def timed(rpc, method, params, times):
    took, size = [], 0
    for _ in range(times):
        _, ms, size = rpc.call(method, params)
        took.append(ms)
    return {"ms": spread(took), "bytes": size}


events = []  # (event, received at ms)
stop = threading.Event()


def follow(seq):
    rpc = Rpc()
    while not stop.is_set():
        try:
            page, _, _ = rpc.call("a11y.since", {"seq": seq, "wait_ms": 2000})
        except RuntimeError:
            # Refused while the person is private: keep the cursor and ask again.
            time.sleep(0.1)
            continue
        received = now_ms()
        for event in page["events"]:
            events.append((event, received))
        seq = page["next_seq"]


acts = []  # (name, ms)


def read_acts(pipe):
    for line in pipe:
        parts = line.split()
        if len(parts) == 3 and parts[0] == "ACT":
            acts.append((parts[1], int(parts[2])))


def act(name):
    acts.append((name, now_ms()))


def first_after(name, wanted, slack_ms=5):
    """The first event after act `name` that `wanted` accepts: (told lag, seen lag) in ms."""
    at = next((ms for act_name, ms in acts if act_name == name), None)
    if at is None:
        return None
    for event, received in events:
        if event["at_ms"] >= at - slack_ms and wanted(event):
            return {"told_after_ms": event["at_ms"] - at, "seen_after_ms": received - at}
    return {"told_after_ms": None, "seen_after_ms": None}


def title(event):
    return ((event.get("window") or {}).get("title") or "") if isinstance(event.get("window"), dict) else ""


def connection_of(connection, pid):
    """The unique name on the accessibility bus of the process `pid`, or None."""
    from gi.repository import Gio, GLib

    bus = ("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus")
    names = connection.call_sync(*bus, "ListNames", None, None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
    for name in names:
        if not name.startswith(":"):
            continue
        try:
            owner = connection.call_sync(*bus, "GetConnectionUnixProcessID", GLib.Variant("(s)", (name,)), None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
        except GLib.Error:
            continue
        if owner == pid:
            return name
    return None


def hostile(rpc, demo, tell):
    """Join the accessibility bus as a stranger and send what a stranger should not be believed
    about: that connections left, that applications left, that another program's window closed.
    Then check the model is still right and still quick.

    Every forgery is addressed to the service under test and to nobody else. A signal with a
    destination is delivered whatever the match rules say, so the test is as good as a broadcast,
    and no other listener on the person's bus (their screen reader, an application's bridge) ever
    sees one."""
    import gi

    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib

    session = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    address = session.call_sync("org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus", "GetAddress", None, None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
    flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
    stranger = Gio.DBusConnection.new_for_address_sync(address, flags, None, None)
    root = "/org/a11y/atspi/accessible/root"
    service = connection_of(stranger, PID)
    if service is None:
        # Never fall back to a broadcast.
        stranger.close_sync(None)
        return {"skipped": "the service under test is not on the accessibility bus"}
    before, _, _ = rpc.call("a11y.now")
    listings_before = before["counts"]["model"]["listings"]

    # (a) A thousand forgeries a second for five seconds: nobody left, and nothing is relisted.
    sent, until = 0, time.time() + 5
    while time.time() < until:
        for _ in range(100):
            stranger.emit_signal(service, "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameOwnerChanged", GLib.Variant("(sss)", (":1.1", ":1.1", "")))
            body = GLib.Variant("(siiva{sv})", ("remove", 0, 0, GLib.Variant("(so)", (":1.1", root)), {}))
            stranger.emit_signal(service, root, "org.a11y.atspi.Event.Object", "ChildrenChanged", body)
            sent += 2
        stranger.flush_sync(None)
        time.sleep(0.1)
    # A dialog in another application, opened while the forgeries pour in, is still told on time.
    tell("dialog", 0.8)
    tell("close-dialog", 0.5)

    # (b) A forged close of the demo's window, from a connection that does not own it.
    result = {"forgeries_sent": sent}
    if demo:
        shown, _, _ = rpc.call("a11y.windows")
        listed = [w for w in shown["windows"] if w["id"] == demo["id"]]
        result["demo_window_listed_before_forged_close"] = bool(listed)
        # Find the demo's own connection and windows the way any program on the bus can, then
        # say, from our connection, that each of its windows was removed.
        owner = connection_of(stranger, demo["pid"])
        forged = 0
        if owner:
            children = stranger.call_sync(owner, root, "org.a11y.atspi.Accessible", "GetChildren", None, None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]
            for child_bus, child_path in children:
                body = GLib.Variant("(siiva{sv})", ("remove", 0, 0, GLib.Variant("(so)", (child_bus, child_path)), {}))
                stranger.emit_signal(service, root, "org.a11y.atspi.Event.Object", "ChildrenChanged", body)
                forged += 1
            stranger.flush_sync(None)
            time.sleep(0.5)
        result["forged_closes_of_the_demo_window"] = forged
    after, _, _ = rpc.call("a11y.now")
    result["listings_during_forgeries"] = after["counts"]["model"]["listings"] - listings_before
    result["demo_window_still_known"] = bool(demo) and any(w["id"] == demo["id"] for w in after["windows"])
    result["now_ms_after"] = timed(rpc, "a11y.now", None, 50)["ms"]
    stranger.close_sync(None)
    return result


def private_time(rpc, tell, demo):
    """Turn Private mode on for the service under test (its own file, not the person's), do
    things in the demo window, turn it off, and check nothing of that time was kept."""
    path = os.environ.get("SIGHT_PRIVACY_FILE")
    if not path:
        return None

    def set_private(on):
        with open(path + ".tmp", "w") as out:
            json.dump({"private": on, "since": int(time.time())}, out)
        os.replace(path + ".tmp", path)

    before, _, _ = rpc.call("a11y.now")
    set_private(True)
    began = now_ms()
    time.sleep(1.5)
    refused = {}
    for method, params in (("a11y.now", None), ("a11y.since", {"seq": 0}), ("a11y.windows", None)):
        try:
            rpc.call(method, params)
            refused[method] = False
        except RuntimeError as why:
            refused[method] = "PRIVATE" in str(why)
    tell("focus-entry", 0.3)
    tell("text typed while private", 0.3)
    tell("focus-button", 0.3)
    tell("title sight-demo: PRIVATE-TITLE", 0.3)
    tell("dialog", 0.6)
    tell("close-dialog", 0.4)
    tell("title sight-demo: afterwards", 0.08)
    set_private(False)
    ended = now_ms()
    time.sleep(1.8)
    after, _, _ = rpc.call("a11y.now")
    inside = [e for e, _ in events if began <= e["at_ms"] <= ended]
    later = [e for e, _ in events if e["at_ms"] > ended]
    return {
        "refused_while_private": refused,
        "events_dated_in_the_private_time": len(inside),
        "events_just_after": [e["kind"] for e in later[:4]],
        "focus_after": after["focus"] if demo and (after["focus"] or {}).get("window") == demo["id"] else None,
        "focus_after_is_null": after["focus"] is None,
        "dropped_private": after["counts"]["dropped_private"] - before["counts"]["dropped_private"],
        "private_title_in_any_event_or_answer": any("PRIVATE-TITLE" in json.dumps(e) for e, _ in events) or "PRIVATE-TITLE" in json.dumps(after),
        "title_after": next((w["title"] for w in after["windows"] if demo and w["id"] == demo["id"]), None),
    }


def main():
    report = {}
    rpc = Rpc()
    start, _, _ = rpc.call("a11y.now")
    report["listening"] = start["listening"]
    report["windows_at_start"] = [f"{w['app']} ({w['role']})" for w in start["windows"]]

    # 1. Idle: what listening costs when nothing happens.
    before = cpu_ms(PID)
    time.sleep(15)
    report["idle"] = {"seconds": 15, "service_cpu_ms": cpu_ms(PID) - before}

    # 1b. The same quiet screen, watched the old way: asking for the windows ten times a second.
    before, asked, until = cpu_ms(PID), 0, time.time() + 15
    while time.time() < until:
        rpc.call("a11y.windows")
        asked += 1
        time.sleep(0.1)
    report["polling_10hz"] = {"seconds": 15, "asked": asked, "service_cpu_ms": cpu_ms(PID) - before}

    reader = threading.Thread(target=follow, args=(start["seq"],), daemon=True)
    reader.start()

    env = dict(os.environ)
    app = subprocess.Popen([sys.executable, APP], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
    threading.Thread(target=read_acts, args=(app.stdout,), daemon=True).start()

    def tell(command, pause=0.8):
        app.stdin.write(command + "\n")
        app.stdin.flush()
        time.sleep(pause)

    time.sleep(2.5)
    # The text field has the focus when the window opens, so the button goes first.
    tell("focus-button")
    tell("focus-entry")
    tell("text Lunch at Rasa SENTINEL-1256")
    tell("title sight-demo: Expenses (edited)")
    tell("dialog", 1.2)
    tell("close-dialog", 1.0)
    act("front-notes")
    subprocess.run(["wlrctl", "toplevel", "focus", "app_id:yantrik-notes"], check=False)
    time.sleep(1.0)
    act("front-demo")
    subprocess.run(["wlrctl", "toplevel", "focus", "title:sight-demo: Expenses (edited)"], check=False)
    time.sleep(1.0)

    # 2. The camera, for comparison: looking when asked.
    shown, _, _ = rpc.call("a11y.now")
    demo = next((w for w in shown["windows"] if w["title"].startswith("sight-demo: Expenses")), None)
    report["now"] = timed(rpc, "a11y.now", None, 300)
    report["windows_pull"] = timed(rpc, "a11y.windows", None, 20)
    if demo:
        report["describe_pull"] = timed(rpc, "a11y.describe", {"window": demo["id"]}, 10)
        listed, _, _ = rpc.call("a11y.windows")
        report["handle_is_stable"] = any(w["id"] == demo["id"] for w in listed["windows"])
        described, _, _ = rpc.call("a11y.describe", {"window": demo["id"]})
        report["describe_summary"] = described["summary"]
    # Recorded only when they are the demo's: if the window manager did not bring it forward,
    # the front window and the focused element are the person's own.
    front_title = (shown.get("front") or {}).get("title") or ""
    report["front_now"] = front_title if front_title.startswith("sight-demo") else None
    report["focus_now"] = shown.get("focus") if demo and (shown.get("focus") or {}).get("window") == demo["id"] else None

    # 3. A firehose: one window changing its text as fast as its toolkit will announce it.
    counts_before = shown["counts"]
    cpu_before, events_before, threads_before = cpu_ms(PID), len(events), threads_cpu_ms(PID)
    tell("burst 2000", 0)
    deadline = time.time() + 30
    while time.time() < deadline and not any(name == "burst-end" for name, _ in acts):
        time.sleep(0.05)
    time.sleep(0.5)
    after, _, _ = rpc.call("a11y.now")
    span = [ms for name, ms in acts if name in ("burst-start", "burst-end")]
    threads_after = threads_cpu_ms(PID)
    report["burst"] = {
        "changes_made": 2000,
        "took_ms": span[1] - span[0] if len(span) == 2 else None,
        "signals_dropped": after["counts"]["signals_dropped"] - counts_before["signals_dropped"],
        "events_told": len(events) - events_before,
        "service_cpu_ms": cpu_ms(PID) - cpu_before,
        "by_thread_ms": {name: ms - threads_before.get(name, 0) for name, ms in threads_after.items() if ms - threads_before.get(name, 0) > 0},
    }

    # 4. A password, typed the way a person types one: nothing about it may be told.
    tell("focus-password", 0.6)
    tell("type-password 12", 0)
    deadline = time.time() + 10
    while time.time() < deadline and not any(name == "password-end" for name, _ in acts):
        time.sleep(0.05)
    time.sleep(0.4)
    typed = [ms for name, ms in acts if name in ("password-start", "password-end")]
    during = [e for e, _ in events if len(typed) == 2 and typed[0] - 5 <= e["at_ms"] <= typed[1] + 300 and e["kind"] == "content_changed" and demo and e.get("window") == demo["id"]]
    report["password"] = {"keystrokes": 12, "content_events_for_its_window": len(during)}

    # 5. A title far longer than any real one is kept short.
    tell("title-long", 0.6)
    long_now, _, _ = rpc.call("a11y.now")
    report["longest_title_bytes"] = max((len(w["title"].encode()) for w in long_now["windows"]), default=0)

    # 6. Signals a program has no business sending (see hostile()).
    report["hostile"] = hostile(rpc, demo, tell)

    # 7. The private time leaves nothing behind.
    report["private"] = private_time(rpc, tell, demo)

    tell("quit", 1.5)
    stop.set()
    reader.join(timeout=4)
    app.wait(timeout=5)

    kind = lambda k: (lambda e: e["kind"] == k)
    report["latency_ms"] = {
        "window opened (from the toolkit mapping it)": first_after("mapped", lambda e: e["kind"] == "window_opened" and title(e).startswith("sight-demo: Expenses")),
        "focus moved to Save": first_after("focus-button", lambda e: e["kind"] == "focus_moved" and e["name"] == "Save"),
        "focus moved to the text field": first_after("focus-entry", lambda e: e["kind"] == "focus_moved" and e["role"] == "text"),
        # Told two seconds after the first key, stamped to the whole second (a text change is
        # one keystroke): the slack is the whole second it is rounded down by.
        "text typed (held 2 s, stamped to the second)": first_after("text", lambda e: e["kind"] == "content_changed" and "text" in e["what"], slack_ms=1000),
        "window retitled": first_after("title", kind("window_renamed")),
        "dialog opened": first_after("dialog", lambda e: e["kind"] == "window_opened" and "Confirm" in title(e)),
        "dialog closed": first_after("close-dialog", kind("window_closed")),
        "another app took the front (wlrctl)": first_after("front-notes", kind("front_changed")),
        "the demo took the front back (wlrctl)": first_after("front-demo", kind("front_changed")),
        "window closed": first_after("quit", lambda e: e["kind"] == "window_closed" and bool(demo) and e["window"]["id"] == demo["id"]),
    }
    # The typed text is in no event and in no answer of the model. (`a11y.describe` returns a
    # field's text by design; that is the looking half, and is not what is checked here.)
    report["typed_text_in_any_event"] = any("SENTINEL-1256" in json.dumps(e) for e, _ in events) or "SENTINEL-1256" in json.dumps(after)
    report["acts"] = acts
    # The demo's own events, whole; anything else by kind and application only.
    demo_ids = {e["window"]["id"] for e, _ in events if e["kind"] == "window_opened" and "sight-demo" in e["window"]["title"]}
    mine = lambda e: "sight-demo" in json.dumps(e) or e.get("app") == "sight-demo-app.py" or (isinstance(e.get("window"), str) and e["window"] in demo_ids)
    report["events"] = [e if mine(e) else {"seq": e["seq"], "at_ms": e["at_ms"], "kind": e["kind"], "app": (e.get("window") or {}).get("app") if isinstance(e.get("window"), dict) else e.get("app")} for e, _ in events]
    report["counts_at_end"] = after["counts"]
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
