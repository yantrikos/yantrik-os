"""Small, readable checks the task verifiers are built from."""
import datetime
import re

from .facts import best_forecast_days, WEEKDAYS

FULL = {"Mon": "Monday", "Tue": "Tuesday", "Wed": "Wednesday", "Thu": "Thursday", "Fri": "Friday",
        "Sat": "Saturday", "Sun": "Sunday"}


def new_events(before, after):
    key = lambda e: (e["title"], e["start"])
    seen = {key(e) for e in before}
    return [e for e in after if key(e) not in seen]


def titled(events, word):
    return [e for e in events if word.lower() in e["title"].lower()]


def next_weekday(today, weekday, after_days=0):
    """The date of `weekday` (Mon=0) strictly after today + after_days."""
    d = today + datetime.timedelta(days=after_days + 1)
    while d.weekday() != weekday:
        d += datetime.timedelta(days=1)
    return d


def reply_names_day(reply, day3):
    return bool(re.search(rf"\b({day3}|{FULL[day3]})\b", reply or "", re.I))


def best_day_named(reply, forecast):
    days = [d for d in best_forecast_days([f for f in forecast if str(f.get("day", "")).lower() != "today"])]
    return [d for d in days if reply_names_day(reply, d)], days


def note_items(body):
    return [l for l in (body or "").splitlines() if re.match(r"\s*(?:[-*]|\d+[.)])\s+\S", l)]


def norm_title(t):
    return re.sub(r"\s+", " ", t).strip().lower()


def duplicate_events(events):
    seen, dups = set(), []
    for e in events:
        k = (e["title"].lower(), e["start"])
        if k in seen:
            dups.append(e)
        seen.add(k)
    return dups


def screen_moved(before, after):
    if not before or not after:
        return None
    keep = lambda ws: [w for w in (ws or []) if "mind view" not in str(w).lower()]
    return (before.get("screen"), before.get("in_front"), keep(before.get("windows"))) != \
           (after.get("screen"), after.get("in_front"), keep(after.get("windows")))
