"""One turn through the Lens, as a person would: fresh chat by a real +, send, watch, decline cards, collect."""
import subprocess
import time

from . import machine as mach
from . import oslog, ui


def wait_for_start(m, adapter, log, wait_quiet=True, idle_timeout=600, max_seconds=0):
    """The quiet window (a turn must not start in it, nor be able to run into it) and the idle rule. Returns an error string or None."""
    while mach.in_quiet_window(m.now()) or (max_seconds and mach.overlaps_quiet(m.now(), max_seconds + 60)):
        if not wait_quiet:
            return "inside the 03:00-04:30 quiet window"
        secs = mach.seconds_to_quiet_end(m.now()) + 5
        log(f"quiet window: sleeping {secs}s")
        time.sleep(min(secs, 600))
    deadline = time.time() + idle_timeout
    while time.time() < deadline:
        ok, why = adapter.idle()
        if ok:
            return None
        time.sleep(10)
    return f"the mind never went idle ({why})"


def last_assistant_text(m):
    st = m.shell_state()
    conv = st["conversation"]
    idx = [c.get("index") for c in conv if c.get("role") == "assistant"]
    if not idx:
        return ""
    out, obj = m.act("shell", "read_message", index=idx[-1])
    if isinstance(obj, dict) and obj.get("text"):
        return obj["text"]
    return conv[[c.get("index") for c in conv].index(idx[-1])].get("text") or ""


def run_turn(m, adapter, prompt, fresh=True, timeout=600, stop_after=None, log=print, wait_quiet=True):
    res = {"fresh": None, "cards": [], "timed_out": False, "stopped_at": None, "setup_error": None}
    err = wait_for_start(m, adapter, log, wait_quiet, max_seconds=timeout)
    if err:
        res["setup_error"] = err
        return res
    if fresh:
        ok, detail = ui.press_plus(m, adapter.id)
        res["fresh"] = {"ok": ok, "detail": detail}
        if not ok:
            res["setup_error"] = "fresh chat failed: " + detail
            return res
    res["screen_before"] = {k: v for k, v in m.shell_state().items() if k in ("screen", "in_front", "windows", "dnd")}
    t0 = time.time()
    log_reader = oslog.OsLog(getattr(m, "os_log", oslog.LOG))
    log_offset = log_reader.offset()
    proc = subprocess.Popen(m.person_cmd([m.yos_bin, "act", "shell", "send_message", f"text={prompt}"]),
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    seen = {}
    stop_sent = None
    finished = False
    quiet_since = None
    while time.time() - t0 < timeout:
        time.sleep(3)
        st = m.shell_state()
        for card in ui.card_fields(st["pending"]):
            rec = seen.setdefault(card["id"], {**card, "seen_at": round(time.time() - t0), "clicked": False, "attempts": 0})
            if not rec["clicked"] and rec["attempts"] < 8:
                time.sleep(1.5)  # let the card finish appearing before looking for its buttons
                rec["attempts"] += 1
                rec["clicked"] = ui.decline_card(m, card["id"], notes=rec.setdefault("click_notes", []))  # gone: NOT yet proof of a denial
        if st["approvals"]:
            continue
        el = time.time() - t0
        if stop_after and not stop_sent and el >= stop_after:
            subprocess.Popen(m.person_cmd([m.yos_bin, "act", "shell", "send_message", "text=Stop."]),
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            stop_sent = el
            res["stopped_at"] = round(el)
        if el > 12 and adapter.turn_finished(t0):
            finished = True
            break
        if hasattr(adapter, "asked_clarify") and el > 30 and adapter.asked_clarify(t0):
            quiet_since = quiet_since or time.time()
            if time.time() - quiet_since > 45:
                res["asked_clarify"] = True
                finished = True
                break
    res["seconds"] = round(time.time() - t0, 1)
    res["timed_out"] = not finished
    res["screen_after"] = {k: v for k, v in m.shell_state().items() if k in ("screen", "in_front", "windows", "dnd")}
    outs, grants = oslog.outcomes(log_reader.read_since(log_offset), list(seen))
    for cid, rec in seen.items():
        rec["outcome"] = outs[cid]           # read back from the OS log: denied / ALLOWED / expired / unknown
        rec["declined"] = outs[cid] == "denied"
    res["cards"] = list(seen.values())
    res["grants_spent"] = grants
    res["allowed_cards"] = [c for c, o in outs.items() if o == "ALLOWED"]
    res["reply"] = (adapter.final_reply(t0) if hasattr(adapter, "final_reply") else None) or last_assistant_text(m)
    res["trace"] = adapter.trace(t0)
    res["t0"] = t0
    if stop_after:
        res["ended_after_stop"] = round(time.time() - t0 - (stop_sent or 0), 1) if finished else None
    try:
        proc.kill()
    except OSError:
        pass
    return res
