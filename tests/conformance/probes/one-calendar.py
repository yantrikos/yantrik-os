#!/usr/bin/env python3
"""There is one calendar.

This machine had two. The Calendar app and the control surface used `calendar-service`, one JSON
file per event under `~/.local/share/yantrik/calendar/`; the built-in companion's own calendar
tools kept a SQLite table of their own, local-first, with optional Google sync. Neither read the
other. "Put it on my calendar", asked of the mind, landed where the app would never show it, and
an appointment made in the app was invisible to the mind's own tools.
`design/calendar-2026-09-20.md` found that and did not fix it; this probe is how the fix is
checked from outside.

It is not a probe of one app. It crosses the two ends deliberately: every event is made by one
side and looked for from the other, and from the files on disk, which are the thing both sides
are supposed to be talking about.

The mind's tools are reached over `companion.sock`, not through a conversation. The shell serves
`companion.tool {name, args}` there (`crates/yantrik-ui/src/companion_rpc.rs`), which runs one
tool by name with no language model in the loop — so what is measured here is the tool, not a
model's willingness to choose it. That helper was local to this file; it is `lib.companion_tool`
now, because the calendar probe wanted it too.

One thing this file used to do and no longer does, because it is the second half of what today's
round fixed. Every read of the app went through a month forward and a month back first: `refresh`
re-read the store only when the visible date range changed, so an event written or removed by
something else while the window sat on one month was not on screen, and a check that read it
without navigating would have been measuring the app's cache rather than the calendar. The window
follows the store now — `describe` asks `calendar.revision` before it answers — so the reads below
are made where they land, and the bound they are given is stated at `FRESHNESS_BOUND_S`.
"""

import datetime
import json
import os
import pathlib
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import lib  # noqa: E402

ONE_JOB = ("There is one calendar: an appointment the mind makes is one the Calendar app shows, "
           "an appointment made in the app is one the mind's own tools can read, and an "
           "appointment removed behind the open window leaves it. The mind's own tools can "
           "neither delete nor edit one: under the shipped ceiling both are refused, and "
           "changing any event is the Calendar's to do, with a person asked (#342).")

APP = "calendar"
APP_BIN = "/opt/yantrik/bin/yantrik-calendar"
SERVICE_BIN = "/opt/yantrik/bin/calendar-service"
STORE = pathlib.Path.home() / ".local/share/yantrik/calendar"
SERVICE_SOCK = lib.SOCKET_DIR / "calendar.sock"
COMPANION_SOCK = lib.COMPANION_SOCK

# Today, because `calendar_today` is one of the two tools that has to see the app's event and it
# only ever looks at today. The store is snapshotted and restored, so a day that already has the
# user's own appointments on it is not a problem.
TODAY = datetime.date.today()

# Titles nothing else on this machine would write.
MIND_TITLE = "conformance-one-calendar-from-the-mind"
APP_TITLE = "conformance-one-calendar-from-the-app"
WHILE_OPEN_TITLE = "conformance-one-calendar-while-the-window-is-open"

# How long an open window may take to show what somebody else wrote, and where the number comes
# from.
#
# Two mechanisms carry it, and this bound is a claim about the first. `describe` asks
# `calendar.revision` before it answers and re-lists the month when that has moved, so the first
# read after a write already carries it: no waiting at all in principle, and five seconds of slack
# for the socket and for the three the control surface allows an action on the UI thread. The
# second is a twenty-second timer in the window, which re-checks the same token while the window
# is visible — that is the fallback for a window nobody is asking about, and it is deliberately
# outside this bound. So a wait here that takes more than five seconds and less than about
# twenty-five means the timer answered and the check before `describe` is not working.
FRESHNESS_BOUND_S = 5


def tool(name, **args):
    """One of the mind's tools, by name. `lib.companion_tool` with the arguments spelled out."""
    return lib.companion_tool(name, args)


# ── The store on disk, which is what both ends are talking about ─────────────

def records():
    """Every event file the calendar store holds, parsed."""
    out = []
    if not STORE.exists():
        return out
    for path in sorted(STORE.glob("*.json")):
        try:
            record = json.loads(path.read_text())
        except (OSError, ValueError):
            continue
        if isinstance(record, dict):
            record["_file"] = path.name
            out.append(record)
    return out


def stored():
    return {r.get("title", r["_file"][:-5]): r for r in records()}


def reported_id(text):
    """The id a create tool answered with, from its `ID: ...` line."""
    for line in (text or "").splitlines():
        if line.startswith("ID:"):
            return line[3:].strip()
    return None


def iso(day):
    return day.isoformat()


def stop_the_service():
    """The calendar service down, and its stale socket gone.

    So that the next thing to need it has to start it, which is the check that "on demand" is a
    mechanism and not a caption. The app is left alone.
    """
    lib.kill_app(SERVICE_BIN)
    for leftover in (SERVICE_SOCK, lib.SOCKET_DIR / "calendar.pid"):
        try:
            leftover.unlink(missing_ok=True)
        except OSError:
            pass


def app_sees():
    """Titles the app publishes for the selected day, and the whole state block beside them.

    No navigation first, and that is the change today's round makes checkable. This used to step
    the month forward and back before every read, because `refresh` re-read the store only when
    the visible range changed and stepping twice was the cheapest way to change it twice — a
    workaround that made every check below a check of the app after being prodded rather than of
    the app. `describe` asks the store whether anything moved before it answers now, so what comes
    back is what the app would tell anyone asking at that moment.

    The state block comes back too, so a check can show what else the app thought was true at the
    same time — the month's count, the days it marked, its notice.
    """
    view = lib.state(APP)
    return [e.get("title") if isinstance(e, dict) else e
            for e in view.get("events_on_selected_day") or []], view


def app_shows(title):
    """Is that title among the events the app publishes for the day it has selected?"""
    return any(title in str(t) for t in app_sees()[0])


def run():
    with lib.Probe("one-calendar", ONE_JOB) as probe:
        # The companion runs one thing at a time, and on every shell start it spends minutes
        # composing a brief. A tool asked for in that window waits its turn, so this probe, run
        # forty seconds after a restart, failed seven checks with "the companion did not finish
        # the tool in time" on code that passed sixteen of sixteen ten minutes later. Waiting for
        # it to be idle measures the calendar instead of the queue. That a mind's tool call can
        # sit behind a brief for minutes is a finding about the companion, recorded here rather
        # than hidden: the wait and how long it took are in the report.
        def companion_idle():
            state = lib.state("shell") or {}
            return state.get("companion_status") == "idle" and not state.get("thinking")

        waited = lib.wait_until(companion_idle, timeout=300,
                                what="the companion to finish what it was doing")
        probe.note("waited_for_companion", waited.evidence())

        probe.note("processes_before", lib.running(APP_BIN) + lib.running(SERVICE_BIN))
        probe.note("store_dir", str(STORE))
        service_was_running = bool(lib.running(SERVICE_BIN))
        app_was_running = bool(lib.running(APP_BIN))
        probe.note("calendar_service_running_before", service_was_running)
        probe.note("calendar_app_running_before", app_was_running)

        store = lib.preserved(STORE)
        with store:
            probe.note("store_before", store.listing())

            # Nothing below may be inherited from a service somebody else started.
            stop_the_service()

            # ── 1. The mind puts something on the calendar ───────────────────
            made = tool("calendar_create_event",
                        summary=MIND_TITLE,
                        start="%sT14:00:00" % iso(TODAY),
                        end="%sT15:00:00" % iso(TODAY))
            probe.check(
                "the mind's calendar_create_event tool can be run at all",
                made["error"] is None,
                contract=1, evidence={"error": made["error"], "socket": str(COMPANION_SOCK)})

            lib.wait_for(lambda: MIND_TITLE in stored(), timeout=20)
            on_disk = stored().get(MIND_TITLE)
            mind_id = reported_id(made["text"])
            probe.check(
                "an event the mind creates is a file in the calendar store, the one the app reads",
                on_disk is not None,
                contract=2, evidence={"answer": made["text"], "titles_on_disk": sorted(stored()),
                                      "store_dir": str(STORE)})
            probe.check(
                "the create tool answers with the id the calendar stored it under, not one of its own",
                bool(mind_id) and on_disk is not None and mind_id == on_disk.get("id"),
                contract=3, evidence={"answered": mind_id,
                                      "id_in_store": (on_disk or {}).get("id"),
                                      "file": (on_disk or {}).get("_file"),
                                      "answer": made["text"]})
            probe.check(
                "the calendar service was started on demand by the tool that needed it",
                bool(lib.running(SERVICE_BIN)) and SERVICE_SOCK.exists(),
                contract=8, evidence={"service_was_running_before": service_was_running,
                                      "service_processes": lib.running(SERVICE_BIN),
                                      "socket": str(SERVICE_SOCK),
                                      "socket_exists": SERVICE_SOCK.exists()})

            # ── 2. The app shows it ──────────────────────────────────────────
            opened = lib.open_app(APP, expect_process=APP_BIN, window_words=("calendar",),
                                  timeout=45)
            probe.check(
                "the Calendar app opens",
                bool(opened["processes"]) and opened["surface_up"],
                contract=1, evidence=opened)

            titles, view = app_sees()
            probe.check(
                "the Calendar app shows the appointment the mind made, on the day it was made for",
                any(MIND_TITLE in str(t) for t in titles)
                and TODAY.day in {d.get("day") for d in view.get("days_with_events") or []
                                  if isinstance(d, dict)},
                contract=2, evidence={"events_on_selected_day": titles,
                                      "days_with_events": view.get("days_with_events"),
                                      "events_this_month": view.get("events_this_month"),
                                      "notice": view.get("notice")})

            # ── 2a. And it keeps up with one made while it is open ───────────
            #
            # The event above was already on disk when the window opened, so showing it only
            # proves the first read. This one is written by the mind while the window sits on
            # this month with nothing touching it, which is the case that used to need a month
            # stepped forward and back before anything would see it. Nothing navigates here.
            while_open = tool("calendar_create_event",
                              summary=WHILE_OPEN_TITLE,
                              start="%sT10:00:00" % iso(TODAY),
                              end="%sT10:30:00" % iso(TODAY))
            lib.wait_for(lambda: WHILE_OPEN_TITLE in stored(), timeout=20)
            appeared = lib.wait_until(
                lambda: app_shows(WHILE_OPEN_TITLE), timeout=FRESHNESS_BOUND_S,
                what="the open window to show an event the mind created behind it")
            probe.check(
                "an open Calendar shows an appointment the mind makes while it is open, inside "
                "%ds, without being navigated" % FRESHNESS_BOUND_S,
                bool(appeared),
                contract=2, evidence=appeared.evidence(
                    tool_answer=while_open["text"], tool_error=while_open["error"],
                    titles_on_disk=sorted(stored()),
                    events_on_selected_day=app_sees()[0],
                    bound_comes_from="describe asks calendar.revision before answering, so the "
                                     "first read after the write should carry it; the window's "
                                     "own twenty-second timer is the fallback and is outside "
                                     "this bound"))

            # ── 3. The app puts something on the calendar ────────────────────
            added = lib.act(APP, "add_event", title=APP_TITLE, date=iso(TODAY), time="16:00")
            lib.wait_for(lambda: APP_TITLE in stored(), timeout=20)
            app_record = stored().get(APP_TITLE)
            probe.check(
                "the app's add_event still writes to the store",
                app_record is not None,
                contract=2, evidence={"action": added.get("result"),
                                      "refused": added.get("refused"),
                                      "titles_on_disk": sorted(stored())})

            # ── 4. And the mind's own tools can read it ──────────────────────
            listed = tool("calendar_list_events", start_date=iso(TODAY), end_date=iso(TODAY))
            probe.check(
                "the mind's calendar_list_events returns the appointment made in the app",
                listed["error"] is None and APP_TITLE in listed["text"],
                contract=2, evidence={"answer": listed["text"], "error": listed["error"],
                                      "id_in_store": (app_record or {}).get("id")})
            probe.check(
                "and it names it by the id the store gave it, which is the id a delete takes",
                app_record is not None and str(app_record.get("id")) in listed["text"],
                contract=3, evidence={"id_in_store": (app_record or {}).get("id"),
                                      "answer": listed["text"]})

            today_answer = tool("calendar_today")
            probe.check(
                "the mind's calendar_today sees both of today's appointments, whichever end made them",
                today_answer["error"] is None
                and APP_TITLE in today_answer["text"] and MIND_TITLE in today_answer["text"],
                contract=2, evidence={"answer": today_answer["text"],
                                      "error": today_answer["error"],
                                      "titles_on_disk": sorted(stored())})

            # ── 5. The mind's tools can neither delete nor edit an event (#342) ──
            #
            # They used to: `calendar_delete_event` and `calendar_update_event` took any event in
            # the store unasked — the person's own, a Google-synced one, another caller's — and a
            # delete cannot be undone. Both are `dangerous` now, above the ceiling the companion
            # ships with, so the registry refuses them before they reach the store. The event the
            # app made is the one tried, because it is the person's, not the mind's.
            app_id = str((app_record or {}).get("id") or "")
            refused_delete = tool("calendar_delete_event", event_id=app_id)
            refused_edit = tool("calendar_update_event", event_id=app_id, summary="taken")
            still = stored().get(APP_TITLE)
            probe.check(
                "the mind's delete and edit of an event it did not make are refused, and the "
                "event stays as it was",
                "Permission denied" in (refused_delete["text"] or "")
                and "Permission denied" in (refused_edit["text"] or "")
                and still is not None and still.get("title") == APP_TITLE,
                contract=4, evidence={"delete_answer": refused_delete["text"],
                                      "delete_error": refused_delete["error"],
                                      "edit_answer": refused_edit["text"],
                                      "edit_error": refused_edit["error"],
                                      "titles_on_disk": sorted(stored())})

            # ── 6. An event removed behind the open window leaves it ─────────
            #
            # This was the mind's delete. The mind cannot delete any more, so the file goes the
            # way any other writer's removal would arrive: from the store, with the window sitting
            # on this month and nothing navigating it. The store is restored on the way out.
            mind_record = stored().get(MIND_TITLE)
            if mind_record:
                (STORE / mind_record["_file"]).unlink(missing_ok=True)
            vanished = lib.wait_until(
                lambda: not app_shows(MIND_TITLE), timeout=FRESHNESS_BOUND_S,
                what="the open window to stop showing an event removed behind it")
            titles_after, view_after = app_sees()
            probe.check(
                "an open Calendar stops showing an event removed behind it inside %ds, without "
                "being navigated" % FRESHNESS_BOUND_S,
                bool(vanished) and not any(MIND_TITLE in str(t) for t in titles_after),
                contract=2, evidence=vanished.evidence(
                    removed_file=(mind_record or {}).get("_file"),
                    events_on_selected_day=titles_after,
                    days_with_events=view_after.get("days_with_events"),
                    notice=view_after.get("notice")))

            # ── Put the machine back ─────────────────────────────────────────
            #
            # `lib.preserved` restores the store as it was found when this block ends: the events
            # this probe made go with it, and nothing here needs a delete the mind may not make.

            # Only what this probe started. A Calendar window the person already had open is
            # theirs, and `open_app` focuses one rather than starting a second.
            if not app_was_running:
                lib.kill_app(APP_BIN)
            if not service_was_running:
                stop_the_service()

        probe.note("store_after", store.listing(store.after))
        probe.check(
            "the calendar store is left exactly as it was found",
            not store.differences(),
            contract="leave-as-found",
            evidence={"before": store.listing(), "after": store.listing(store.after),
                      "differences": store.differences() or "none"})

        leftover = lib.running(APP_BIN) + lib.running(SERVICE_BIN)
        probe.note("processes_after", leftover)
        # Which programs, not which pids. The service is on demand: this probe stops it to prove
        # a tool can start it, and something starts it again afterwards, so a service that was
        # running before and is running after is the machine as it was found — under a new pid.
        # Comparing the listings whole failed this check on a machine that was exactly as found.
        def programs(listing):
            return sorted({line.split(None, 1)[1] if " " in line else line for line in listing})

        probe.check(
            "no calendar process is left running that was not running before",
            set(programs(leftover)) <= set(programs(probe.notes["processes_before"])),
            contract="leave-as-found",
            evidence={"before": probe.notes["processes_before"], "after": leftover})

        probe.note("companion_socket_helper", {
            "where": "lib.companion_tool",
            "what": "newline-delimited JSON-RPC over %s, calling companion.tool" % COMPANION_SOCK,
            "moved": "it was local to this probe. probes/calendar.py is the second caller, which "
                     "is the condition the old note set for moving it — the two copies of "
                     "verify_calendar.py are why lib.py exists.",
        })

        probe.note("how_the_window_follows_the_store", {
            "bound_asserted_s": FRESHNESS_BOUND_S,
            "mechanism": "calendar-service answers `calendar.revision` — the event count and the "
                         "newest modification time under the store — and the app asks it before "
                         "every describe and on a twenty-second timer while its window is "
                         "visible, re-listing the month only when the token has moved.",
            "what_this_probe_no_longer_does": "step the month forward and back before every read. "
                                              "`refresh` re-read only when the visible range "
                                              "changed, so that was the cheapest way to change it "
                                              "twice — and it made every check here a check of "
                                              "the app after being prodded.",
            "what_would_show_the_check_before_describe_had_stopped_working":
                "a wait that settles between %ds and about 25s: that is the timer answering "
                "rather than the revision check." % FRESHNESS_BOUND_S,
        })


if __name__ == "__main__":
    run()
