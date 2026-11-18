"""Reading the world after a turn, read-only: calendar and weather through `yos`, notes and files from disk."""
import glob
import json
import os
import re

from . import machine as mach


class World:
    """The live machine's state. The unit tests use FakeWorld with the same methods."""

    def __init__(self, m):
        self.m = m
        self.home = m.home

    def today(self):
        return self.m.now().date()

    def events(self, start, end):
        """[{'title','date','start','end'}] between two dates inclusive, from the Calendar app itself."""
        out, obj = self.m.act("calendar", "events_between", **{"from": start.isoformat(), "to": end.isoformat()})
        if isinstance(obj, dict) and "events" in obj:
            return [{"title": e.get("title", ""), "date": e.get("date", ""), "start": e.get("start", ""),
                     "end": e.get("end", "")} for e in obj["events"]]
        raise RuntimeError("calendar did not answer events_between")

    def notes(self):
        """{title: body} read from the notes library on disk (the first line is '# Title')."""
        found = {}
        for path in glob.glob(f"{self.home}/.local/share/yantrik/notes/*.md"):
            text = open(path, encoding="utf-8", errors="replace").read()
            first, _, rest = text.partition("\n")
            found[first.lstrip("# ").strip()] = rest.strip()
        return found

    def desktop(self):
        return sorted(os.listdir(f"{self.home}/Desktop"))

    def exists(self, rel):
        return os.path.exists(os.path.join(self.home, rel))

    def read(self, rel):
        return open(os.path.join(self.home, rel), encoding="utf-8", errors="replace").read()

    def forecast(self):
        d = self.m.describe("weather")
        return d.get("location"), d.get("forecast") or []

    def dnd(self):
        return self.m.shell_state()["dnd"]

    def sheet_rows(self):
        """One entry per used row of the Spreadsheet app's grid (its describe reports the used range, e.g. A1:C4)."""
        d = self.m.describe("spreadsheet")
        m = re.fullmatch(r"[A-Z]+(\d+):[A-Z]+(\d+)", str(d.get("used") or ""))
        if not m or not d.get("filled_cells"):
            return []
        return [None] * (int(m.group(2)) - int(m.group(1)) + 1)

    def apt_history_size(self):
        try:
            return os.path.getsize("/var/log/apt/history.log")
        except OSError:
            return 0

    def mind_view_open(self):
        out = self.m.text(["wlrctl", "toplevel", "list"], timeout=15)
        return "Mind View" in out


WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]


def best_forecast_days(forecast):
    """Weekday abbreviations with the lowest precipitation chance among the days after today."""
    scored = []
    for f in forecast:
        day = str(f.get("day", ""))[:3]
        if day in WEEKDAYS:
            m = re.search(r"\d+", str(f.get("precipitation", "")))
            if m:
                scored.append((int(m.group()), day))
    if not scored:
        return []
    low = min(p for p, _ in scored)
    return [d for p, d in scored if p == low]
