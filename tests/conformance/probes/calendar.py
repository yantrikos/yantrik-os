#!/usr/bin/env python3
"""Calendar's one job: keep an appointment.

An event you add is written to the calendar store on disk, is shown by the app, and is
still there when the app comes back — and when it cannot be kept, the calendar says so
instead of answering success.

Every claim below is checked against the store on disk or against a process the probe did
not start, never against the action's own answer. The audit's finding was exactly that
answer: `add_event` returned `{"added": "audit", "on": "..."}` for an event that existed
on no day. `design/calendar-2026-09-20.md` has the four faults underneath it.

The same fabrication was one layer up in the other two views until 617dac9. `week-events`,
`day-events`, `week-day-labels` and `day-view-title` were `in` properties nothing in the
repository ever set, so pressing Week showed a grid with no events and blank column
headers while `set_view` answered `{"view": "week"}` as though it had shown something.
Both views now answer as themselves, and the checks for them work the same way as every
other check here: the week's range, its column labels and the number of blocks on it are
worked out from the store on disk by this file, and the app's answer is compared against
that — never the other way round.

Deleting used to be the thing this file could not measure. The control surface published
nothing that removed an event — `delete-event` was a Slint callback taking a row index,
reachable only from the window — so the data-loss bug 617dac9 fixed, where every row of a
day carried the id 0 and the trash icon on any of them deleted the first, was NOT EXERCISED
and said so in a note. `delete_event` and `update_event` are on the surface now and go
through the same path the trash icon does, so the checks below are the ones that note
listed as missing: two events on one day, the one asked for removed and the other still in
the store on disk.

`delete_event` is graded `sensitive`, which this machine's ceiling allows — only
`dangerous` is refused by policy here. It is still classified with `lib.refusal_kind`
rather than assumed: if the ceiling is ever tightened, a policy refusal is not the app
declining, and the honest record is that the check was not exercised.
"""

import datetime
import json
import os
import pathlib
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import lib  # noqa: E402

ONE_JOB = ("Keep an appointment: an event you add is on disk, is shown by the app in "
           "whichever of its three views is up, and is still there after a restart.")

APP = "calendar"
APP_BIN = "/opt/yantrik/bin/yantrik-calendar"
SERVICE_BIN = "/opt/yantrik/bin/calendar-service"
STORE = pathlib.Path.home() / ".local/share/yantrik/calendar"
SERVICE_SOCK = lib.SOCKET_DIR / "calendar.sock"

# The days this probe works on, in the month the machine is in. Days of the month rather
# than a written-out date, because `select_day` picks a day of the month on screen and the
# app opens on today's: a fixed 2026-09-24 stops meaning anything on the first of October.
# Every month has a 24th, a 25th and a 26th, so these three always exist.
#
# The 24th carries the event the week and day views are checked on, the 26th the 23:30
# clamp, the 25th the refusal, and the last day of the month the event that has to show up
# in the neighbouring month's first week. The titles are ones nothing else here would write.
TODAY = datetime.date.today()
DATE = TODAY.replace(day=24)
LATE_DATE = TODAY.replace(day=26)
REFUSED_DATE = TODAY.replace(day=25)
TITLE = "conformance-launch-review"
SECOND_TITLE = "conformance-morning-standup"
LATE_TITLE = "conformance-evening-walk"
NEIGHBOUR_TITLE = "conformance-month-edge"
REFUSED_TITLE = "conformance-should-not-exist"

# The delete pair. Both go on the 24th, between the 09:00 and the 14:00 already there, so the
# one that is removed is not the first event of its day — which is the whole of the bug: the
# trash icon on any row deleted the first. DOOMED is deleted by its id and KEPT must survive it.
DOOMED_TITLE = "conformance-delete-this-one"
KEPT_TITLE = "conformance-leave-this-one"
# Two events of one name on one day, which is an ordinary thing for a calendar to hold and the
# reason `delete_event` refuses a title it cannot resolve to exactly one event.
AMBIGUOUS_TITLE = "conformance-two-of-these"
NO_SUCH_ID = "conformance-no-such-event-id"

# Written out rather than taken from `strftime`, which answers in the machine's locale. The
# app builds these names from tables of its own in `views.rs`, and a probe that asked the C
# library for them would be checking one table against another table's language.
SHORT_DAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
LONG_DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]
MONTHS = ["January", "February", "March", "April", "May", "June",
          "July", "August", "September", "October", "November", "December"]


def iso(day):
    return day.isoformat()


def records():
    """Every event file the store holds, parsed. The witness for point 2."""
    out = []
    if not STORE.exists():
        return out
    for path in sorted(STORE.glob("*.json")):
        try:
            record = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        if not isinstance(record, dict):
            continue
        record["_file"] = path.name
        out.append(record)
    return out


def stored():
    """The same, as {title: record}, for the checks that ask after one event by name."""
    return {r.get("title", r["_file"][:-5]): r for r in records()}


# ── The arithmetic the two time views are checked against ─────────────────────
#
# Done here, from the dates and from what is on disk, so that `set_view`'s answer and
# `describe`'s account of the week are compared with something this probe worked out. The
# week starts on Sunday: `views.rs` says so, because the month grid's header row does, and
# a week view starting anywhere else would put one date in two columns of one calendar.

def week_bounds(day):
    """The Sunday on or before `day`, and the Saturday six days after it."""
    start = day - datetime.timedelta(days=(day.weekday() + 1) % 7)
    return start, start + datetime.timedelta(days=6)


def last_day_of_month(day):
    first_of_next = (day.replace(day=28) + datetime.timedelta(days=4)).replace(day=1)
    return first_of_next - datetime.timedelta(days=1)


def column_label(day):
    """How the week view heads that day's column: `Thu 24`."""
    return "%s %d" % (SHORT_DAYS[(day.weekday() + 1) % 7], day.day)


def day_title(day):
    """How the day view names itself: `Thursday, 24 September 2026`."""
    return "%s, %d %s %d" % (LONG_DAYS[(day.weekday() + 1) % 7], day.day,
                             MONTHS[day.month - 1], day.year)


def month_title(day):
    return "%s %d" % (MONTHS[day.month - 1], day.year)


def parse_stamp(text):
    """What the store keeps, in the forms `views::parse_datetime` accepts, or None.

    `fromisoformat` rather than `strptime`, and not for taste. `strptime` imports the
    standard library's `_strptime` the first time it is called, and `_strptime` imports
    `calendar` — and this file is named `calendar.py` and sits in the directory Python puts
    first on the path when the runner starts it. So `import calendar` found *this probe*
    and ran the whole of it a second time inside the first, which arrived as a bare
    `SystemExit: 1` with nothing to say for itself. The guard at the foot of this file is
    the other half of that lesson.
    """
    try:
        return datetime.datetime.fromisoformat(str(text))
    except ValueError:
        return None


def blocks_between(first, last):
    """How many blocks the events on disk put on a grid covering `first..=last`.

    One block per day an event touches, which is how an event running past midnight is
    drawn; an all-day event has no hour and is not on the hour grid at all. This is the
    number `set_view` and `describe` have to agree with, and it is counted from the files
    rather than read off the app.
    """
    count = 0
    for record in records():
        if record.get("is_all_day"):
            continue
        start = parse_stamp(record.get("start"))
        end = parse_stamp(record.get("end"))
        if start is None or end is None or end < start:
            continue
        day = max(start.date(), first)
        through = min(end.date(), last)
        while day <= through:
            day_start = datetime.datetime.combine(day, datetime.time())
            next_day = day_start + datetime.timedelta(days=1)
            minutes = (min(end, next_day) - max(start, day_start)).total_seconds() / 60
            if minutes > 0 or start == end:
                count += 1
            day += datetime.timedelta(days=1)
    return count


def events_in_month(day):
    prefix = "%04d-%02d" % (day.year, day.month)
    return sum(1 for r in records() if str(r.get("start", "")).startswith(prefix))


def straddle_case(from_month):
    """The month edge to check, as (how many months forward, the day to open the week view
    on, the day in the neighbouring month that week also holds).

    The scenario is the one the bug was found in: an event on the last day of a month, and
    the week view opened on the first of the next, which is where 28 to 30 September used
    to be drawn empty. The edge is searched for rather than assumed — when the 1st of a
    month falls on a Sunday no week crosses that boundary at all, and a 28-day February
    beginning on a Sunday has no crossing week at either of its edges — so the next edge
    along is tried until one straddles. Two in a row can happen; three cannot.
    """
    edge = last_day_of_month(from_month)
    for step in range(4):
        first_of_next = edge + datetime.timedelta(days=1)
        if week_bounds(first_of_next)[0] <= edge:
            return step + 1, first_of_next, edge
        edge = last_day_of_month(first_of_next)
    # Unreachable on the Gregorian calendar, and a straddle that has to be searched four
    # months for is worth failing over rather than skipping.
    return 1, last_day_of_month(from_month) + datetime.timedelta(days=1), \
        last_day_of_month(from_month)


def column_for(week, day):
    """The column of the week `describe` published that belongs to `day`, or None.

    Matched on the label this probe computed for that date, as a prefix: a column with an
    all-day event on it carries a count after the date (`Thu 24 · 1 all day`), and a column
    that is missing entirely has to be told apart from one that is empty.
    """
    want = column_label(day)
    for column in week or []:
        if isinstance(column, dict) and str(column.get("day", "")).startswith(want):
            return [e for e in column.get("events") or [] if isinstance(e, dict)]
    return None


def block_for(blocks, title):
    return next((b for b in blocks or [] if str(b.get("title")) == title), None)


def stop_everything():
    """The app and the service down, and the service's stale socket gone."""
    lib.kill_app(APP_BIN)
    lib.kill_app(SERVICE_BIN)
    for leftover in (SERVICE_SOCK, lib.SOCKET_DIR / "calendar.pid"):
        try:
            leftover.unlink(missing_ok=True)
        except OSError:
            pass


def open_calendar():
    return lib.open_app(APP, expect_process=APP_BIN, window_words=("calendar",), timeout=45)


def run():
    """The probe itself. In a function, and called only when this file is the script.

    `probes/` is first on `sys.path` while the runner has this file open, so `import
    calendar` anywhere in this process — the standard library's own `_strptime` does
    it — finds this probe and not the module it wanted. Run at import, the body would
    open a second calendar, add a second set of events and kill the app the first one
    was still using. Imported, this file now does nothing.
    """
    with lib.Probe(APP, ONE_JOB) as probe:
        probe.note("processes_before", lib.running(APP_BIN) + lib.running(SERVICE_BIN))
        probe.note("windows_before", lib.toplevels())

        store = lib.preserved(STORE)
        with store:
            probe.note("store_before", store.listing())

            # A cold machine, so nothing below can be inherited from an earlier run.
            stop_everything()

            # ── 1. It opens ───────────────────────────────────────────────────
            opened = open_calendar()
            probe.check(
                "it opens: a process exists and the compositor has its window",
                bool(opened["processes"]) and bool(opened["windows"]) and opened["surface_up"],
                contract=1, evidence=opened)
            probe.check(
                "a launch that worked adds nothing to the shell's failed_launches",
                opened["new_failed_launches_for_this_app"] == [],
                contract=1, evidence={"added_by_this_launch": opened["new_failed_launches"],
                                      "whole_list": opened["failed_launches"]})

            # ── 2. It does its one job, and the store agrees ──────────────────
            before_titles = sorted(stored())
            added = lib.act(APP, "add_event", title=TITLE, date=iso(DATE), time="14:00")
            lib.wait_for(lambda: TITLE in stored(), timeout=15)
            after = stored()
            record = after.get(TITLE)
            probe.check(
                "add_event writes the event to the store on disk",
                record is not None,
                contract=2, evidence={"titles_before": before_titles, "titles_after": sorted(after),
                                      "record": record, "store_dir": str(STORE)})

            probe.check(
                "the service it needs is started on demand, not assumed",
                bool(lib.running(SERVICE_BIN)) and SERVICE_SOCK.exists(),
                contract=8, evidence={"service_processes": lib.running(SERVICE_BIN),
                                      "socket": str(SERVICE_SOCK),
                                      "socket_exists": SERVICE_SOCK.exists()})

            # ── 3. The action reports what happened ───────────────────────────
            stored_id = (record or {}).get("id")
            answered_id = (added.get("result") or {}).get("id")
            probe.check(
                "add_event answers with the id it was stored under",
                bool(answered_id) and answered_id == stored_id,
                contract=3, evidence={"action_result": added.get("result"),
                                      "id_in_store": stored_id,
                                      "file_in_store": (record or {}).get("_file"),
                                      "accepted": added.get("accepted"),
                                      "settled": added.get("settled")})

            # ── 2 again, from the app's side: describe must agree with the disk ──
            lib.act(APP, "select_day", day=DATE.day)
            time.sleep(1)
            view = lib.state(APP)
            days = {d.get("day"): d.get("events") for d in view.get("days_with_events") or []
                    if isinstance(d, dict)}
            on_day = [e.get("title") if isinstance(e, dict) else e
                      for e in view.get("events_on_selected_day") or []]
            probe.check(
                "describe shows the event the store holds",
                view.get("events_this_month", 0) >= 1 and DATE.day in days
                and any(TITLE in str(t) for t in on_day),
                contract=2, evidence={"events_this_month": view.get("events_this_month"),
                                      "days_with_events": view.get("days_with_events"),
                                      "events_on_selected_day": on_day,
                                      "titles_on_disk": sorted(after)})

            # A 23:30 event used to build an end of T24:30:00, which is not a time.
            lib.act(APP, "add_event", title=LATE_TITLE, date=iso(LATE_DATE), time="23:30")
            lib.wait_for(lambda: LATE_TITLE in stored(), timeout=15)
            late = stored().get(LATE_TITLE) or {}
            end = str(late.get("end", ""))
            probe.check(
                "a late event is clamped to a real time, not given a 24:30 end",
                bool(end) and "T24:" not in end and end <= iso(LATE_DATE) + "T23:59:59",
                contract=3, evidence={"start": late.get("start"), "end": late.get("end")})

            # ── 2 and 3 in the other two views ────────────────────────────────
            #
            # Week and Day were an empty drawing: a grid with no events, blank column headers and
            # a "now" line on midnight, while `set_view` answered `{"view": "week"}`. So what is
            # asked of them here is what was asked of `add_event` — say what is on screen, and be
            # checked against the store rather than against the answer.
            #
            # A second event on the same day, earlier in it: the day grid has to put two things on
            # one day in the order they happen, and one thing at one time proves neither.
            lib.act(APP, "add_event", title=SECOND_TITLE, date=iso(DATE), time="09:00")
            lib.wait_for(lambda: SECOND_TITLE in stored(), timeout=15)

            week_from, week_to = week_bounds(DATE)
            blocks_this_week = blocks_between(week_from, week_to)
            lib.act(APP, "select_day", day=DATE.day)
            week = lib.act(APP, "set_view", view="week")
            answered = week.get("result") or {}
            probe.check(
                "set_view week answers with the week on screen — its range and how much is on it —"
                " not the word it was given",
                answered.get("view") == "week"
                and answered.get("week_start") == iso(week_from)
                and answered.get("week_end") == iso(week_to)
                and answered.get("events") == blocks_this_week and blocks_this_week >= 1,
                contract=3, evidence={"answer": answered, "summary": week.get("summary"),
                                      "week_this_probe_worked_out": [iso(week_from), iso(week_to)],
                                      "blocks_the_store_implies": blocks_this_week})

            time.sleep(1)
            view = lib.state(APP)
            seen_labels = [str(c.get("day", "")) for c in view.get("week") or []
                           if isinstance(c, dict)]
            want_labels = [column_label(week_from + datetime.timedelta(days=i)) for i in range(7)]
            probe.check(
                "the week's seven columns are headed with their own dates, not left blank",
                len(seen_labels) == 7
                and all(seen.startswith(want) for seen, want in zip(seen_labels, want_labels)),
                contract=2, evidence={"labels": seen_labels, "dates_this_probe_worked_out":
                                      want_labels})

            its_day = column_for(view.get("week"), DATE)
            block = block_for(its_day, TITLE)
            probe.check(
                "describe in week view reports the range and lists the event under its own day, "
                "at its time and for its length",
                view.get("view") == "week"
                and view.get("week_start") == iso(week_from)
                and view.get("week_end") == iso(week_to)
                and block is not None and block.get("at") == "14:00"
                and block.get("minutes") == 60,
                contract=2, evidence={"view": view.get("view"),
                                      "range": [view.get("week_start"), view.get("week_end")],
                                      "column": column_label(DATE), "on_that_column": its_day,
                                      "the_block": block,
                                      "in_the_store": (stored().get(TITLE) or {}).get("start")})

            blocks_this_day = blocks_between(DATE, DATE)
            day = lib.act(APP, "set_view", view="day")
            answered = day.get("result") or {}
            probe.check(
                "set_view day answers with the day it moved to and how much is on it",
                answered.get("view") == "day"
                and str(answered.get("day", "")).startswith(day_title(DATE))
                and answered.get("events") == blocks_this_day and blocks_this_day >= 2,
                contract=3, evidence={"answer": answered, "summary": day.get("summary"),
                                      "day_this_probe_worked_out": day_title(DATE),
                                      "blocks_the_store_implies": blocks_this_day})

            time.sleep(1)
            view = lib.state(APP)
            grid = [b for b in view.get("events_on_day_grid") or [] if isinstance(b, dict)]
            times = [str(b.get("at")) for b in grid]
            titles = [str(b.get("title")) for b in grid]
            probe.check(
                "describe in day view names the day and lists what is on it, in time order",
                view.get("view") == "day"
                and str(view.get("day_shown", "")).startswith(day_title(DATE))
                and TITLE in titles and times == sorted(times),
                contract=2, evidence={"view": view.get("view"), "day_shown": view.get("day_shown"),
                                      "grid": grid, "times_in_the_order_given": times})

            # The half of the trash-icon bug that can be seen from out here: a day holding two
            # events has to arrive as two entries, each with its own time. The row index the
            # delete handler is given is a position in exactly this list.
            early = block_for(grid, SECOND_TITLE)
            late_block = block_for(grid, TITLE)
            probe.check(
                "two events on one day arrive as two entries, each with its own time",
                early is not None and late_block is not None
                and early.get("at") == "09:00" and late_block.get("at") == "14:00"
                and titles.index(SECOND_TITLE) < titles.index(TITLE),
                contract=2, evidence={"grid": grid,
                                      "on_disk": {t: (stored().get(t) or {}).get("start")
                                                  for t in (SECOND_TITLE, TITLE)}})

            month = lib.act(APP, "set_view", view="month")
            answered = month.get("result") or {}
            in_month = events_in_month(DATE)
            probe.check(
                "switching back to month answers as the month again",
                answered.get("view") == "month"
                and answered.get("showing") == month_title(DATE)
                and answered.get("events") == in_month,
                contract=3, evidence={"answer": answered, "summary": month.get("summary"),
                                      "month_this_probe_worked_out": month_title(DATE),
                                      "events_the_store_holds_this_month": in_month})
            time.sleep(1)
            view = lib.state(APP)
            probe.check(
                "and describe goes back to the month with it: no week range, no day grid",
                view.get("view") == "month" and "week" not in view
                and "week_start" not in view and "events_on_day_grid" not in view,
                contract=3, evidence={"view": view.get("view"), "keys": sorted(view)})

            # ── The week that runs across a month boundary ────────────────────
            #
            # The app asked the store for exactly the month on screen, so the first week of
            # October drew 28 to 30 September empty and said nothing about it. The event goes on
            # the last day of a month and the week view is opened on the first of the next one.
            steps, anchor, neighbour = straddle_case(DATE)
            lib.act(APP, "add_event", title=NEIGHBOUR_TITLE, date=iso(neighbour), time="10:00")
            lib.wait_for(lambda: NEIGHBOUR_TITLE in stored(), timeout=15)
            for _ in range(steps):
                lib.act(APP, "show_month", direction="next")
            lib.act(APP, "select_day", day=anchor.day)
            edge_from, edge_to = week_bounds(anchor)
            blocks_on_the_edge = blocks_between(edge_from, edge_to)
            straddle = lib.act(APP, "set_view", view="week")
            answered = straddle.get("result") or {}
            time.sleep(1)
            view = lib.state(APP)
            neighbour_column = column_for(view.get("week"), neighbour)
            neighbour_block = block_for(neighbour_column, NEIGHBOUR_TITLE)
            probe.check(
                "a week that runs across a month boundary shows the neighbouring month's event "
                "instead of an empty column",
                answered.get("week_start") == iso(edge_from)
                and answered.get("week_end") == iso(edge_to)
                and answered.get("events") == blocks_on_the_edge
                and neighbour_block is not None and neighbour_block.get("at") == "10:00",
                contract=2, evidence={
                    "answer": answered,
                    "week_this_probe_worked_out": [iso(edge_from), iso(edge_to)],
                    "blocks_the_store_implies": blocks_on_the_edge,
                    "the_event_is_on": iso(neighbour), "the_week_was_opened_on": iso(anchor),
                    "months_this_week_spans": sorted({edge_from.month, edge_to.month}),
                    "months_stepped_forward": steps,
                    "column": column_label(neighbour), "on_that_column": neighbour_column,
                    "the_whole_week": view.get("week")})

            # Back where the checks below expect to find it: this month, month view, the 24th.
            lib.act(APP, "go_to_today")
            lib.act(APP, "set_view", view="month")
            lib.act(APP, "select_day", day=DATE.day)

            # ── The trash icon that deleted the wrong event ───────────────────
            #
            # `events_for_day` gave every row of a day the id 0, the screen passes `event.id` to
            # `delete-event` and the handler uses it as an index, so the trash icon on any row of
            # a day deleted the first event on it (617dac9). Until today that could not be checked
            # from out here: `delete-event` is a Slint callback taking a row index and the control
            # surface published nothing that removed anything, so this section was a note saying
            # NOT EXERCISED. `delete_event` now goes through the same path the trash icon does.
            surface = lib.actions(APP)
            probe.check(
                "the control surface publishes a way to take an event off the calendar",
                "delete_event" in surface and "update_event" in surface
                and "update_own_event" in surface,
                contract=2, evidence={"actions": surface})

            # Two more on the 24th, between the 09:00 and the 14:00 already on it. The one that
            # is deleted is second of four by time, so a handler that still indexed from zero
            # would take the 09:00 and this would fail on the event that survived.
            lib.act(APP, "add_event", title=DOOMED_TITLE, date=iso(DATE), time="11:00")
            lib.act(APP, "add_event", title=KEPT_TITLE, date=iso(DATE), time="12:00")
            lib.wait_for(lambda: DOOMED_TITLE in stored() and KEPT_TITLE in stored(), timeout=15)
            doomed = (stored().get(DOOMED_TITLE) or {}).get("id")
            kept_before = stored().get(KEPT_TITLE) or {}

            removed = lib.act(APP, "delete_event", id=doomed or "")
            how_refused = lib.refusal_kind(removed)
            if how_refused == "policy":
                # The ceiling refused on the grade before the app was asked, so nothing below
                # would be the app's account of anything. See README, "The third outcome".
                probe.note("delete_not_exercised", {
                    "delete_path_exercised": False,
                    "statement": "Deleting an event was NOT EXERCISED on this machine. A green "
                                 "result for calendar is not evidence that the trash icon "
                                 "removes the event it is sitting on.",
                    "why": "`calendar.delete_event` is graded `sensitive` and this machine's "
                           "ceiling is below it, so the control surface refused on the grade "
                           "alone, before dispatch. The app never ran.",
                    "the_refusal_in_full": removed.get("refused"),
                    "checks_not_exercised": [
                        "delete_event by id removes the event it was asked for",
                        "every other event of that day is still in the store on disk",
                        "an ambiguous title is refused with its candidates",
                        "an unknown id is refused",
                    ],
                    "what_was_still_measured": "that two events on one day arrive as two "
                                               "separate entries in time order — the list the "
                                               "deleted row is an index into",
                    "actions_the_surface_publishes": surface,
                })
            else:
                gone = lib.wait_until(
                    lambda: DOOMED_TITLE not in stored(), timeout=15,
                    what="the deleted event to leave the store on disk")
                after_delete = stored()
                probe.check(
                    "delete_event removes the event whose id it was given",
                    bool(gone) and DOOMED_TITLE not in after_delete,
                    contract=2, evidence=gone.evidence(
                        answer=removed.get("result"), refused=removed.get("refused"),
                        id_asked_for=doomed, titles_on_disk=sorted(after_delete)))
                probe.check(
                    "and the other events of that day are still in the store on disk — the "
                    "regression 617dac9 fixed, where the trash icon on any row deleted the first",
                    KEPT_TITLE in after_delete
                    and (after_delete.get(KEPT_TITLE) or {}).get("id") == kept_before.get("id")
                    and SECOND_TITLE in after_delete and TITLE in after_delete,
                    contract=2, evidence={
                        "deleted": DOOMED_TITLE, "deleted_id": doomed,
                        "id_of_the_one_that_had_to_survive": kept_before.get("id"),
                        "its_id_now": (after_delete.get(KEPT_TITLE) or {}).get("id"),
                        "titles_on_disk": sorted(after_delete),
                        "the_day_in_time_order": ["09:00 " + SECOND_TITLE,
                                                  "11:00 " + DOOMED_TITLE + " (deleted)",
                                                  "12:00 " + KEPT_TITLE,
                                                  "14:00 " + TITLE]})
                probe.check(
                    "the action answers with what it removed, not with what it was asked to",
                    (removed.get("result") or {}).get("deleted") == DOOMED_TITLE
                    and (removed.get("result") or {}).get("id") == doomed,
                    contract=3, evidence={"answer": removed.get("result"),
                                          "accepted": removed.get("accepted"),
                                          "settled": removed.get("settled")})

                # A title that names two events on one day. Never a guess: picking either would
                # remove the wrong appointment half the time and report success.
                lib.act(APP, "add_event", title=AMBIGUOUS_TITLE, date=iso(DATE), time="15:00")
                lib.act(APP, "add_event", title=AMBIGUOUS_TITLE, date=iso(DATE), time="16:00")
                lib.wait_for(
                    lambda: len([r for r in records()
                                 if r.get("title") == AMBIGUOUS_TITLE]) == 2, timeout=15)
                twins = [r for r in records() if r.get("title") == AMBIGUOUS_TITLE]
                twin_ids = sorted(str(r.get("id")) for r in twins)
                titles_before_ambiguity = sorted(stored())
                ambiguous = lib.act(APP, "delete_event", title=AMBIGUOUS_TITLE, date=iso(DATE))
                refusal = str(ambiguous.get("refused") or "")
                probe.check(
                    "a title that names two events on one day is refused, with both candidates "
                    "and their ids, rather than one of them being guessed at",
                    lib.refusal_kind(ambiguous) == "app"
                    and all(i in refusal for i in twin_ids)
                    and "15:00" in refusal and "16:00" in refusal,
                    contract=4, evidence={"refusal": refusal,
                                          "refused_by": lib.refusal_kind(ambiguous),
                                          "the_two_on_disk": twin_ids})
                probe.check(
                    "and nothing was removed while it was refusing",
                    sorted(stored()) == titles_before_ambiguity,
                    contract=4, evidence={"titles_before": titles_before_ambiguity,
                                          "titles_after": sorted(stored())})

                phantom = lib.act(APP, "delete_event", id=NO_SUCH_ID)
                probe.check(
                    "deleting an id that is not on this machine is refused in words naming it",
                    lib.refusal_kind(phantom) == "app" and NO_SUCH_ID in str(phantom.get("refused")),
                    contract=4, evidence={"refusal": phantom.get("refused"),
                                          "refused_by": lib.refusal_kind(phantom)})

            # ── Moving an appointment ─────────────────────────────────────────
            #
            # The contract has had `update_event` all along and nothing on the surface could
            # change an appointment, so a mind could put something on this calendar at the wrong
            # time and had to delete it and make it again. The check is the file on disk, not the
            # action's answer: KEPT_TITLE runs 12:00-13:00, and moving it to 15:30 must keep the
            # hour it already runs for, because the instruction said nothing about length.
            # KEPT_TITLE is this probe's own event, so it moves through `update_own_event`
            # (standard): `update_event` reaches any event and is `sensitive` since #332, which
            # in ask mode would leave this section refused on its grade and never exercised.
            moved_id = (stored().get(KEPT_TITLE) or {}).get("id")
            moved = lib.act(APP, "update_own_event", id=moved_id or "", time="15:30")
            how_refused = lib.refusal_kind(moved)
            if how_refused == "policy":
                probe.note("update_not_exercised", {
                    "update_path_exercised": False,
                    "statement": "Moving an event was NOT EXERCISED on this machine.",
                    "why": "the control surface refused `update_own_event` on its grade, before "
                           "dispatch. The app never ran.",
                    "the_refusal_in_full": moved.get("refused"),
                    "checks_not_exercised": ["update_own_event moves an event and the file on disk "
                                             "agrees, keeping the length it already ran for"],
                })
            else:
                landed = lib.wait_until(
                    lambda: str((stored().get(KEPT_TITLE) or {}).get("start", "")).endswith(
                        "T15:30:00"),
                    timeout=15, what="the moved event to be at its new time on disk")
                record = stored().get(KEPT_TITLE) or {}
                probe.check(
                    "update_own_event moves the probe's own event on disk, and keeps how long it runs",
                    bool(landed)
                    and record.get("start") == iso(DATE) + "T15:30:00"
                    and record.get("end") == iso(DATE) + "T16:30:00"
                    and record.get("id") == moved_id,
                    contract=2, evidence=landed.evidence(
                        answer=moved.get("result"), refused=moved.get("refused"),
                        on_disk={"id": record.get("id"), "start": record.get("start"),
                                 "end": record.get("end")},
                        it_used_to_run="12:00 to 13:00, one hour"))
                probe.check(
                    "and answers with what the store holds now, read back after the write",
                    (moved.get("result") or {}).get("start") == record.get("start")
                    and (moved.get("result") or {}).get("end") == record.get("end"),
                    contract=3, evidence={"answer": moved.get("result"),
                                          "on_disk": {"start": record.get("start"),
                                                      "end": record.get("end")}})

            # ── 6. It survives a restart ──────────────────────────────────────
            killed = lib.kill_app(APP_BIN)
            reopened = open_calendar()
            time.sleep(1)
            after_restart = lib.state(APP)
            on_disk_after_restart = sorted(stored())
            probe.check(
                "what was made is still there after the app is killed and reopened",
                bool(reopened["processes"])
                and after_restart.get("events_this_month", 0) >= 2
                and TITLE in on_disk_after_restart and LATE_TITLE in on_disk_after_restart,
                contract=6, evidence={"killed": killed,
                                      "reopened_processes": reopened["processes"],
                                      "reopened_windows": reopened["windows"],
                                      "events_this_month": after_restart.get("events_this_month"),
                                      "titles_on_disk": on_disk_after_restart})

            # ── 4. Failure is said twice ──────────────────────────────────────
            # The only way to check a refusal is to make saving impossible. One file in
            # /opt/yantrik/bin is renamed, inside a context manager that puts it back on the
            # way out, on an exception, on SIGTERM and from an atexit hook.
            stop_everything()
            titles_before_failure = sorted(stored())
            try:
                with lib.moved_aside(SERVICE_BIN):
                    failure_open = open_calendar()
                    refused = lib.act(APP, "add_event", title=REFUSED_TITLE,
                                      date=iso(REFUSED_DATE), time="09:00")
                    time.sleep(1)
                    notice = lib.state(APP).get("notice") or ""
                    titles_after_failure = sorted(stored())
            except (FileNotFoundError, RuntimeError) as exc:
                probe.check("the failure case could be set up", False, contract=4,
                            evidence={"error": str(exc),
                                      "note": "needs passwordless sudo to rename one binary"})
                refused, notice, titles_after_failure = {}, "", titles_before_failure
                failure_open = {}

            probe.check(
                "with the service gone, add_event is refused in words the caller can read",
                refused.get("accepted") is False and bool(refused.get("refused"))
                and refused.get("refused") not in ("1", "0"),
                contract=4, evidence={"refusal": refused.get("refused"),
                                      "accepted": refused.get("accepted"),
                                      "opened_for_failure_case": failure_open.get("processes")})
            probe.check(
                "nothing was written while it was refusing",
                titles_after_failure == titles_before_failure,
                contract=4, evidence={"titles_before": titles_before_failure,
                                      "titles_after": titles_after_failure})
            probe.check(
                "the same failure is in describe.notice, not only in the caller's error",
                bool(notice.strip()),
                contract=4, evidence={"notice": notice, "refusal": refused.get("refused")})

            probe.check(
                "the binary that was renamed away is back",
                pathlib.Path(SERVICE_BIN).exists()
                and not pathlib.Path(SERVICE_BIN + ".conformance-hidden").exists(),
                contract="leave-as-found",
                evidence={SERVICE_BIN: pathlib.Path(SERVICE_BIN).exists()})

            # ── Put the machine back ──────────────────────────────────────────
            stop_everything()

        probe.note("store_after", store.listing(store.after))
        probe.check(
            "the calendar store is left exactly as it was found",
            not store.differences(),
            contract="leave-as-found",
            evidence={"before": store.listing(), "after": store.listing(store.after),
                      "differences": store.differences() or "none"})

        # Put the service back if it was up when this probe started. It stops the service to
        # prove that a calendar which cannot reach its store says so, and until the companion's
        # tools began starting it at shell start the service was never up beforehand, so stopped
        # WAS as found. It is not any more, and leaving a machine with its calendar service down
        # because a test ran is the thing leave-as-found exists to prevent.
        def programs(listing):
            return sorted({line.split(None, 1)[1] if " " in line else line for line in listing})

        was_up = any(SERVICE_BIN in line for line in probe.notes["processes_before"])
        if was_up and not lib.running(SERVICE_BIN):
            restarted = lib.act("shell", "start_service", name="calendar")
            lib.wait_until(lambda: bool(lib.running(SERVICE_BIN)), timeout=5,
                           what="the calendar service to come back up")
            probe.note("service_restored", {"asked_the_shell": restarted.get("result")
                                            or restarted.get("refused")})

        # And the window, if somebody had it open. This put the service back and not the app, so
        # a run started while Calendar was on screen closed it and then failed itself for having
        # done so — on a machine where every check of the calendar had passed.
        app_was_open = any(APP_BIN in line for line in probe.notes["processes_before"])
        if app_was_open and not lib.running(APP_BIN):
            lib.open_app(APP, expect_process=APP_BIN)

        leftover = lib.running(APP_BIN) + lib.running(SERVICE_BIN)
        probe.note("processes_after", leftover)
        probe.note("windows_after", lib.toplevels())
        # Which programs, not which pids: a service stopped and started again is as found.
        probe.check(
            "the calendar's processes are as they were found: the same programs running, no more and no fewer",
            programs(leftover) == programs(probe.notes["processes_before"]),
            contract="leave-as-found",
            evidence={"before": probe.notes["processes_before"], "after": leftover})


if __name__ == "__main__":
    run()
