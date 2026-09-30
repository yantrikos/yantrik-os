#!/usr/bin/env python3
"""Build the Daily Briefing dashboard.

Pulls live data from this desktop's own apps (weather, calendar, email, notes,
system-monitor, shell) with `yos`, and writes a single self-contained HTML file:

    ~/Documents/daily-briefing/index.html

Run it again any time to refresh the numbers:

    python3 ~/Documents/daily-briefing/build_briefing.py
"""

import datetime
import html
import json
import os
import subprocess
from string import Template

YOS = "/opt/yantrik/bin/yos"
HERE = os.path.expanduser("~/Documents/daily-briefing")
OUT = os.path.join(HERE, "index.html")


def sh(*args):
    """Run a command and hand back everything it printed."""
    try:
        p = subprocess.run(list(args), capture_output=True, text=True, timeout=90)
        return (p.stdout or "") + (p.stderr or "")
    except Exception as exc:  # a surface that is asleep should not kill the page
        return "ERROR: %s" % exc


def describe(app):
    """Ask an app what it is showing, and return its state as a dict."""
    text = sh(YOS, "describe", app)
    start = text.find("{")
    if start < 0:
        return {}
    try:
        obj, _ = json.JSONDecoder().raw_decode(text[start:])
        return obj if isinstance(obj, dict) else {}
    except Exception:
        return {}


def act(app, action, **kwargs):
    """Ask an app to do something, e.g. act('calendar', 'select_day', day=25)."""
    args = ["%s=%s" % (k, v) for k, v in kwargs.items()]
    return sh(YOS, "act", app, action, *args)


def esc(value):
    return html.escape(str(value if value is not None else "")),


def n(value):
    if isinstance(value, (int, float)):
        return value
    try:
        return float(value)
    except Exception:
        return 0.0


# ---------------------------------------------------------------- gather ----

now = datetime.datetime.now()
today = now.date()

weather = describe("weather")
system = describe("system-monitor")
notes = describe("notes")
email = describe("email")
shell = describe("shell")

# Calendar: put it back on today, read today, then walk the days this month
# that hold something and collect their events.
act("calendar", "go_to_today")
cal = describe("calendar")
today_events = cal.get("events_on_selected_day", []) or []

upcoming = []
for entry in cal.get("days_with_events", []) or []:
    day = int(entry.get("day", 0))
    if day <= today.day:
        continue
    act("calendar", "select_day", day=day)
    day_state = describe("calendar")
    events = day_state.get("events_on_selected_day", []) or []
    if events:
        upcoming.append((day, events))
    if len(upcoming) >= 4:
        break
act("calendar", "go_to_today")

# ---------------------------------------------------------------- shape ------

WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]

hour = now.hour
if hour < 12:
    greeting = "Good morning"
elif hour < 18:
    greeting = "Good afternoon"
else:
    greeting = "Good evening"


def event_rows(events):
    if not events:
        return '<li class="empty">Nothing on the calendar.</li>'
    rows = []
    for ev in events:
        when = ev.get("time") or ""
        if ev.get("all_day"):
            when = "all day"
        rows.append(
            '<li><span class="when">%s</span><span class="what">%s</span></li>'
            % (html.escape(when or "—"), html.escape(str(ev.get("title", ""))))
        )
    return "".join(rows)


def upcoming_blocks():
    if not upcoming:
        return '<p class="empty">No later events this month.</p>'
    out = []
    for day, events in upcoming:
        stamp = datetime.date(today.year, today.month, day)
        delta = (stamp - today).days
        label = "%s %d · in %d day%s" % (
            WEEKDAYS[stamp.weekday()], day, delta, "" if delta == 1 else "s"
        )
        out.append('<div class="upday"><h4>%s</h4><ul>%s</ul></div>' % (label, event_rows(events)))
    return "".join(out)


messages = email.get("messages", []) or []
unread = [m for m in messages if not m.get("read")]
mail_rows = []
for msg in unread[:6]:
    mail_rows.append(
        '<li><span class="from">%s</span><span class="what">%s</span><span class="when">%s</span></li>'
        % (
            html.escape(str(msg.get("from", ""))),
            html.escape(str(msg.get("subject", ""))[:88]),
            html.escape(str(msg.get("date", ""))[5:16]),
        )
    )
mail_rows = "".join(mail_rows) or '<li class="empty">Inbox is clear.</li>'

note_rows = []
for note in (notes.get("notes", []) or [])[:7]:
    if note.get("trash"):
        continue
    badge = ' <em class="pin">pinned</em>' if note.get("pinned") else ""
    note_rows.append(
        '<li><span class="what">%s</span>%s</li>'
        % (html.escape(str(note.get("title", ""))), badge)
    )
note_rows = "".join(note_rows) or '<li class="empty">No notes yet.</li>'

disks = system.get("disks") or [{}]
disk = disks[0] if disks else {}
mem_used = system.get("memory_used", "")
mem_total = system.get("memory_total", "")

notif = shell.get("notifications") or {}
mode = shell.get("mind_mode") or {}
net = shell.get("network") or {}


def stat(value, label, sub=""):
    return (
        '<div class="stat"><div class="big">%s</div><div class="lab">%s</div>'
        '<div class="sub">%s</div></div>'
        % (html.escape(str(value)), html.escape(label), html.escape(str(sub)))
    )


stats = "".join([
    stat("%d%%" % int(n(system.get("cpu_percent"))), "CPU", "load %.2f" % n((system.get("load") or [0])[0])),
    stat("%d%%" % int(n(system.get("memory_percent"))), "Memory", "%s / %s" % (mem_used, mem_total)),
    stat("%d%%" % int(n(disk.get("percent"))), "Disk", "%s / %s" % (disk.get("used", "?"), disk.get("total", "?"))),
    stat(notif.get("unread", 0), "Unread notices", "%d in total" % int(n(notif.get("showing")))),
])

stamp = now.strftime("%A, %d %B %Y · %H:%M")

# ---------------------------------------------------------------- render ----

TEMPLATE = Template("""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Daily Briefing — $date_short</title>
<style>
  :root {
    --bg: #0a0e13;
    --panel: #121821;
    --line: #1f2937;
    --ink: #e6edf3;
    --dim: #8b9aab;
    --accent: #22d3ee;
    --warm: #fbbf24;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 34px 30px 60px;
    background: radial-gradient(1100px 500px at 12% -8%, #12304347, transparent), var(--bg);
    color: var(--ink);
    font: 15px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, Roboto, sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  header { max-width: 1180px; margin: 0 auto 26px; }
  .kicker { color: var(--accent); font-size: 12px; letter-spacing: .18em; text-transform: uppercase; font-weight: 700; }
  h1 { margin: 8px 0 4px; font-size: 34px; font-weight: 650; letter-spacing: -.02em; }
  .sub { color: var(--dim); font-size: 14px; }
  .clock { font-variant-numeric: tabular-nums; color: var(--ink); }
  .grid {
    max-width: 1180px; margin: 0 auto;
    display: grid; gap: 16px;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  }
  .card {
    background: linear-gradient(180deg, #141b25, var(--panel));
    border: 1px solid var(--line); border-radius: 14px; padding: 18px 18px 16px;
  }
  .card.wide { grid-column: 1 / -1; }
  h3 {
    margin: 0 0 12px; font-size: 12px; letter-spacing: .14em;
    text-transform: uppercase; color: var(--accent); font-weight: 700;
  }
  h4 { margin: 14px 0 6px; font-size: 13px; color: var(--dim); font-weight: 600; }
  ul { list-style: none; margin: 0; padding: 0; }
  li { padding: 5px 0; border-bottom: 1px dashed #1b2430; display: flex; gap: 10px; align-items: baseline; }
  li:last-child { border-bottom: 0; }
  .when { color: var(--warm); font-variant-numeric: tabular-nums; font-size: 13px; min-width: 88px; }
  .from { color: var(--accent); font-size: 13px; min-width: 92px; }
  .what { flex: 1; }
  .pin { color: var(--warm); font-size: 11px; font-style: normal; }
  .empty { color: var(--dim); font-style: italic; }
  .weather { display: flex; align-items: flex-start; gap: 18px; }
  .temp { font-size: 46px; font-weight: 300; line-height: 1; letter-spacing: -.03em; }
  .wmeta { color: var(--dim); font-size: 13px; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 12px; }
  .stat { background: #0d131b; border: 1px solid var(--line); border-radius: 11px; padding: 12px 13px; }
  .big { font-size: 23px; font-weight: 600; }
  .lab { color: var(--accent); font-size: 10.5px; letter-spacing: .12em; text-transform: uppercase; margin-top: 2px; }
  .sub { font-size: 12px; color: var(--dim); }
  .upday ul { margin-top: 2px; }
  footer { max-width: 1180px; margin: 26px auto 0; color: var(--dim); font-size: 12px; }
  code { background: #0d131b; border: 1px solid var(--line); border-radius: 5px; padding: 1px 5px; color: var(--accent); }
</style>
</head>
<body>
<header>
  <div class="kicker">Daily briefing</div>
  <h1>$greeting, Alex.</h1>
  <div class="sub">$date_long &nbsp;·&nbsp; <span class="clock" id="clock">$time</span> &nbsp;·&nbsp; $location</div>
</header>

<div class="grid">

  <div class="card">
    <h3>Outside</h3>
    <div class="weather">
      <div class="temp">$temp</div>
      <div class="wmeta">
        $condition<br>
        feels $feels &nbsp;·&nbsp; humidity $humidity%<br>
        wind $wind $winddir &nbsp;·&nbsp; UV $uv
      </div>
    </div>
  </div>

  <div class="card">
    <h3>Today</h3>
    <ul>$today_rows</ul>
  </div>

  <div class="card">
    <h3>Inbox — $unread of $total unread</h3>
    <ul>$mail_rows</ul>
  </div>

  <div class="card">
    <h3>Notes — $note_count in the library</h3>
    <ul>$note_rows</ul>
  </div>

  <div class="card wide">
    <h3>This machine</h3>
    <div class="stats">$stats</div>
  </div>

  <div class="card wide">
    <h3>Coming up</h3>
    $upcoming
  </div>

</div>

<footer>
  Gathered from this desktop's own apps — weather, calendar, email, notes and
  system-monitor — at $stamp.<br>
  Refresh with <code>python3 ~/Documents/daily-briefing/build_briefing.py</code>
  · network: $net &nbsp;·&nbsp; mind mode: $mode
</footer>

<script>
  function tick() {
    var el = document.getElementById('clock');
    if (!el) return;
    var d = new Date();
    el.textContent = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }
  tick();
  setInterval(tick, 1000);
</script>
</body>
</html>
""")

page = TEMPLATE.substitute(
    date_short=today.strftime("%d %b %Y"),
    date_long="%s %d %s" % (WEEKDAYS[today.weekday()], today.day, today.strftime("%B %Y")),
    greeting=greeting,
    time=now.strftime("%H:%M"),
    location=html.escape(str(weather.get("location") or "location unknown")),
    temp=("%.0f°" % n(weather.get("temperature"))) if weather else "—",
    condition=html.escape(str(weather.get("condition") or "—")),
    feels="%.0f°" % n(weather.get("feels_like")),
    humidity=int(n(weather.get("humidity"))),
    wind="%.0f" % n(weather.get("wind_speed")),
    winddir=html.escape(str(weather.get("wind_direction") or "")),
    uv="%.1f" % n(weather.get("uv_index")),
    today_rows=event_rows(today_events),
    upcoming=upcoming_blocks(),
    mail_rows=mail_rows,
    unread=int(n((email.get("counts") or {}).get("unread"))),
    total=int(n((email.get("counts") or {}).get("total"))),
    note_count=int(n(notes.get("note_count"))),
    note_rows=note_rows,
    stats=stats,
    stamp=stamp,
    net=html.escape(str(net.get("connection") or "unknown")),
    mode=html.escape(str(mode.get("mode") or "unknown")),
)

os.makedirs(HERE, exist_ok=True)
with open(OUT, "w", encoding="utf-8") as fh:
    fh.write(page)

print("WROTE %s (%d bytes)" % (OUT, len(page)))
print("today_events=%d upcoming_days=%d unread_mail=%d notes=%d"
      % (len(today_events), len(upcoming), len(unread), int(n(notes.get("note_count")))))
