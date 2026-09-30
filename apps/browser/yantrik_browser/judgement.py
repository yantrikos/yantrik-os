"""Whether a press is a commitment: the word list, OR the person's decision model.

The word list (commit.py) knows plain English. It misses "Continue" on a payment step, an
icon-only Send, "OK" on a transfer, and every other language: recall 0.375 on the 102 cases in
tools/judge-eval, against about 0.9 for a System One model. So when the words say nothing, the
decision model chosen in Settings is asked, through the shell's `decide`
(crates/yantrik-ui/src/control_decide.rs).

It can only add a card, never remove one:
- a control the words call a commitment stays one, whatever the model says;
- the model answering yes at THRESHOLD or above makes one of a control the words passed;
- an abstention, a timeout, no model, or the use switched off in Settings leaves the words'
  answer as it was.

A link that only navigates is not asked about, because Back undoes a navigation. The threshold
is low on purpose: a false alarm costs the person one card, a miss costs money.
"""

import threading
import time
import urllib.parse

from yantrik_surface import gate, wire

THRESHOLD = 0.4          # tools/judge-eval: word list OR Kev at 0.4 had recall 1.0 locally
BUDGET = 3.0             # seconds a press waits for the model; past it, the words decide
CACHE_FOR = 600.0        # seconds a judgement of one control on one site is reused
CACHE_SIZE = 512

QUESTION = (
    "Would activating this control, on this page, do something that reaches past the browser and "
    "cannot simply be undone: spend or move money, place an order, send or publish something to "
    "other people, delete data, or grant another app or person access? Navigating, searching, "
    "filtering, opening a form or dialog, adding to a cart, starting a checkout and saving a draft "
    "do not count."
)


def _ask_shell(state):
    """The shell's `decide` for one control; the reply object."""
    path = wire.default_socket_path(gate.SHELL)
    return wire.call_once(path, "app.act", {
        "action": "decide",
        "args": {
            "purpose": "browser_commitment",
            "state": state,
            "questions": {"commit": {"type": "noul", "instructions": QUESTION}},
        },
    }, timeout=BUDGET, peer_rule=gate.must_be_the_shell)


def where_of(url):
    """The page's address without its query or fragment: where the page is, not what the address
    carries — sign-in codes and reset tokens ride in those, and a cloud model is not sent them."""
    try:
        u = urllib.parse.urlsplit(url or "")
        return urllib.parse.urlunsplit((u.scheme, u.hostname or "", u.path, "", ""))[:300]
    except ValueError:
        return ""


def yes_in(reply):
    """The probability of yes in the shell's answer, and who gave it; (None, reason) when the
    model abstained or the answer is not a verdict."""
    result = (reply or {}).get("result") if isinstance(reply, dict) else None
    verdict = result.get("result") if isinstance(result, dict) else None
    if not isinstance(verdict, dict):
        error = (reply or {}).get("error") if isinstance(reply, dict) else None
        return None, (error or {}).get("message", "no verdict") if isinstance(error, dict) else "no verdict"
    answer = (verdict.get("answers") or {}).get("commit") or {}
    by = verdict.get("by") or {}
    who = ("%s %s" % (by.get("provider", "?"), by.get("model", ""))).strip()
    if answer.get("type") != "noul" or not isinstance(answer.get("yes"), (int, float)):
        return None, answer.get("reason") or "abstained"
    return float(answer["yes"]), who


class Judgement:
    """The decision model's view of controls, cached, asked only when the words say nothing."""

    def __init__(self, ask=None):
        self.ask = ask or _ask_shell
        self.cache = {}
        self.lock = threading.Lock()

    def state_of(self, control, url, title):
        return {
            "control": {"role": control.get("role") or "clickable", "label": control.get("name") or "(no label: an icon)"},
            "page": {"title": (title or "")[:200], "url": where_of(url), "heading": (control.get("heading") or "")[:200]},
            "nearby_text": (control.get("nearby") or "")[:400],
        }

    def commitment(self, control, url, title):
        """A sentence naming why the model thinks pressing `control` is a commitment, or None."""
        if control.get("role") == "link" and control.get("href") and not control.get("scripted"):
            return None
        state = self.state_of(control, url, title)
        host = (urllib.parse.urlsplit(url or "").hostname or "").lower()
        key = (host, state["control"]["role"], state["control"]["label"], state["page"]["heading"], state["nearby_text"])
        now = time.monotonic()
        with self.lock:
            hit = self.cache.get(key)
        if hit and now - hit[2] < CACHE_FOR:
            p, who = hit[0], hit[1]
        else:
            try:
                p, who = yes_in(self.ask(state))
            except (OSError, ConnectionError, ValueError, wire.PeerRefused) as e:
                p, who = None, str(e)
            # Only an answer is remembered. A model that was busy, slow or refused is asked again
            # next time: remembering "no answer" for ten minutes let whoever kept it busy once
            # leave a control unchecked for all of them (security review, 29 Sep 2026).
            if p is not None:
                with self.lock:
                    if len(self.cache) >= CACHE_SIZE:
                        self.cache.pop(next(iter(self.cache)))
                    self.cache[key] = (p, who, now)
        if p is not None and p >= THRESHOLD:
            return "judged a commitment by %s, p = %.2f" % (who, p)
        return None
