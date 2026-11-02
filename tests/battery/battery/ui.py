"""What a person does with the mouse, found without hard-coded coordinates, and checked after.

- press_plus: the Lens's + (New chat). Only a real press begins a clean taint scope, so a control-surface
  `new_chat` is not good enough; the runner confirms the scope id changed and fails the setup if it did not.
- cards: detected from describe, recorded field by field, declined by clicking Decline. Never Allow.
- pick_mind: the Lens's mind picker.
"""
import json
import time

from . import pixels


def taint_scope(machine, harness):
    path = f"{machine.home}/.local/share/yantrik/taint-scopes.json"
    try:
        data = json.load(open(path))["scopes"].get(harness, {})
    except (OSError, ValueError, KeyError):
        return {}
    return {"id": data.get("id"), "begun": data.get("begun"), "read": data.get("read", [])}


def ensure_lens_open(machine):
    machine.act("shell", "open_lens")
    time.sleep(1.5)


def press_plus(machine, harness, tries=3):
    """A real pointer press on the Lens's +. Returns (ok, detail)."""
    before = taint_scope(machine, harness)
    for attempt in range(tries):
        ensure_lens_open(machine)
        shot = machine.grim_ppm()
        if not shot:
            return False, "no screenshot (grim)"
        w, h, data = shot
        icons = pixels.find_lens_header_icons(w, h, data)
        if len(icons) < 3:
            time.sleep(1)
            continue
        machine.click(*icons[0])
        time.sleep(2)
        after = taint_scope(machine, harness)
        if after.get("id") and after.get("id") != before.get("id") and not after.get("read"):
            return True, f"scope {before.get('id')} -> {after.get('id')} begun={after.get('begun')} reads=[]"
    return False, f"scope id did not change after pressing + (before={before}); tests would run in a tainted scope"


# -- cards -----------------------------------------------------------------------------------------
def card_fields(pending):
    out = []
    for c in pending:
        v = c.get("verified") or {}
        out.append({"id": c.get("id"), "action": c.get("action"), "app": c.get("app"), "grade": c.get("grade"),
                    "requester": c.get("requester"), "attached_mind": v.get("attached_mind"),
                    "agent": v.get("agent"), "line": v.get("line")})
    return out


def decline_card(machine, card_id, settle=3):
    """Click Decline on the card; True when that card is gone afterwards. Never clicks Allow."""
    for _ in range(3):
        shot = machine.grim_ppm()
        if not shot:
            return False
        w, h, data = shot
        spot = pixels.find_card_buttons(w, h, data)
        if not spot:
            time.sleep(1)
            continue
        if pixels.looks_like_allow(w, h, data, *spot):
            return False  # never click where the button looks like Allow/Run
        machine.click(*spot)
        time.sleep(settle)
        if not any(c.get("id") == card_id for c in machine.shell_state()["pending"]):
            return True
    return False


# -- the mind picker -----------------------------------------------------------------------------------
def pick_mind(machine, want, names):
    """Make `want` ('mind' or 'hermes') the answering mind through the Lens's picker. names: id -> picker label.

    The picker's list order and row heights vary, so rows are tried from the top and the result is checked
    with describe each time. Returns (ok, detail)."""
    if want in machine.shell_state()["answering"]:
        return True, "already answering"
    for attempt in range(8):
        if machine.shell_state()["pending"]:
            return False, "an approval card is up; not clicking the picker blind"
        ensure_lens_open(machine)
        shot = machine.grim_ppm()
        if not shot:
            return False, "no screenshot"
        w, h, _ = shot
        machine.click(w - int(w * 0.268), h - int(h * 0.124))  # the picker chip, bottom left of the Lens bar
        time.sleep(1)
        # candidate row centres, from the top of the list (measured at 1280x800: 407 452 511 556 601 646)
        rows = [407, 452, 511, 556, 601, 646]
        y = int(rows[attempt % len(rows)] * h / 800)
        machine.click(w - int(w * 0.30), y)
        time.sleep(3)
        if want in machine.shell_state()["answering"]:
            return True, f"selected on try {attempt + 1}"
    return False, "could not select the mind through the picker; select it by hand and rerun"
