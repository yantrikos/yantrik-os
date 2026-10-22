#!/usr/bin/env python3
"""Weather's one job: remember the places someone saved, and say when it cannot find one.

Two faults are under test here.

`WeatherState::save()` was defined and never called, while `new()` read the file back — so the
round trip looked whole and every saved city and the choice of degrees vanished at the next
launch. Nothing short of killing the app and reopening it proves that is fixed, and the file on
disk is checked as well as what the window says, because the window could be holding the right
value for the wrong reason.

`add_location` answered `{"added": name, "saved": N}` whether or not the geocoder found anything.
So a name that cannot resolve must come back as a refusal — and as a refusal a caller can read.
This machine may have no internet, which is why nothing here requires a *successful* lookup: a
lookup that fails has to say so, and that is the assertion either way.

This probe was written before `lib.py` and carried its own `act()` with the bug `lib` exists to
fix: `yos` reports a refusal by printing to stderr and exiting, so `str(SystemExit(1))` is the
string "1", and every refusal in this report read "1" — including the one this file's whole third
section is about. It is ported now, and the refusal is asserted on as text: the app's own words,
naming the place it could not find, and told to the person in `describe.notice` as well. Whether
the refusal came from the app or from the machine's ceiling is read off the answer with
`lib.refusal_kind` rather than guessed at from the sentence; a ceiling refusal would satisfy "it
did not answer success" while proving nothing about the app, so where one happens the checks about
the app's own words are recorded as NOT EXERCISED instead of passed.
"""

import json
import os
import pathlib
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import lib  # noqa: E402

ONE_JOB = ("Remember the places someone saved and the scale they chose, across a restart — and "
           "when a place cannot be looked up, say so instead of answering that it was added.")

APP = "weather"
APP_BIN = "/opt/yantrik/bin/yantrik-weather"
SERVICE_BIN = "/opt/yantrik/bin/weather-service"
# The framework's own place for an app's settings (frameworks/yantrik-app/src/paths.rs); the
# file is the envelope {"kind", "app", "schema", "settings"}.
CONFIG = pathlib.Path.home() / ".config/yantrik/apps/weather/settings.json"
APP_SOCK = lib.SOCKET_DIR / "app-weather.sock"
NOWHERE = "Zzxqvnowhereville"

OUTCOME = {
    "policy": "refused by policy — the machine's ceiling turned it away before the app saw it",
    "app": "refused by the app",
    None: "answered by the app",
}


def on_disk():
    """The prefs as the next launch will read them, or why they could not be read."""
    try:
        doc = json.loads(CONFIG.read_text())
    except FileNotFoundError:
        return {}
    except Exception as exc:  # noqa: BLE001 - an unreadable file is a result, not a crash
        return {"unreadable": "%s: %s" % (type(exc).__name__, exc)}
    settings = doc.get("settings") if isinstance(doc, dict) else None
    return settings if isinstance(settings, dict) else {"unreadable": "not the framework's settings envelope"}


def saved_places(prefs=None):
    prefs = on_disk() if prefs is None else prefs
    places = prefs.get("locations") if isinstance(prefs, dict) else None
    return [p for p in places or [] if isinstance(p, dict)]


def stop_app():
    lib.kill_app(APP_BIN)
    # A socket file outlives the process it belonged to, so leaving it would make the wait for
    # the surface return at once against a window that is not there.
    try:
        APP_SOCK.unlink(missing_ok=True)
    except OSError:
        pass


def open_weather():
    return lib.open_app(APP, expect_process=APP_BIN, window_words=("weather",), timeout=45)


def run():
    """The probe. In a function so that importing this file does nothing — see calendar.py."""
    not_exercised = []

    with lib.Probe(APP, ONE_JOB) as probe:
        was_running = bool(lib.running(APP_BIN))
        probe.note("processes_before", lib.running(APP_BIN))

        config = lib.preserved(CONFIG)
        with config:
            probe.note("config_before", {"existed": CONFIG.exists(), "prefs": on_disk()})
            before_bytes = CONFIG.read_bytes() if CONFIG.exists() else None

            stop_app()

            # Everything from here is inside a try, so that the app this probe opened is
            # closed and the prefs are put back even when an assertion above throws.
            try:
                # ── 1. It opens ───────────────────────────────────────────────
                opened = open_weather()
                probe.check(
                    "it opens: a process exists, the compositor has its window, and the surface "
                    "answers",
                    bool(opened["processes"]) and bool(opened["windows"]) and opened["surface_up"],
                    contract=1, evidence=opened)
                probe.check(
                    "this launch added nothing to the shell's failed_launches",
                    not opened["new_failed_launches_for_this_app"],
                    contract=1, evidence={"added_by_this_launch": opened["new_failed_launches"]})

                # Reading Open-Meteo directly when the service does not answer is a fallback
                # worth having, and on a healthy machine it is a fault worth failing on. Every
                # autostart service here was once killed 200 ms after the shell started it, and
                # this probe stayed green for the whole day because the app fetched for itself
                # and nothing asked where the reading had come from.
                service_up = lib.running(SERVICE_BIN)
                settled = lib.wait_until(
                    lambda: (lib.state(APP).get("reading_from") or "").startswith(
                        ("weather-service", "open-meteo")),
                    timeout=30, what="a first reading to arrive from somewhere")
                reading_from = lib.state(APP).get("reading_from")
                probe.check(
                    "the weather service the shell starts is up, and it is what the app is reading",
                    bool(service_up) and reading_from == "weather-service",
                    contract=8, evidence={"reading_from": reading_from,
                                          "service_processes": service_up,
                                          "waited": settled.evidence()})

                opening = lib.state(APP)
                probe.note("on_open", {"units": opening.get("units"),
                                       "reading_from": opening.get("reading_from"),
                                       "notice": opening.get("notice"),
                                       "saved_locations": opening.get("saved_locations")})
                # Where the numbers came from is part of the contract: a reading fetched directly
                # because the service was down must not read as one the service produced.
                probe.check(
                    "describe says where the reading came from",
                    bool(opening.get("reading_from")),
                    contract=3, evidence={"reading_from": opening.get("reading_from")})
                probe.check(
                    "describe carries a notice field, so a failure can be said to the caller too",
                    "notice" in opening,
                    contract=4, evidence={"notice": opening.get("notice"), "keys": sorted(opening)})

                # ── 2. A choice that has to outlive the process ───────────────
                was = opening.get("units")
                want = "celsius" if was == "fahrenheit" else "fahrenheit"
                set_units = lib.act(APP, "set_units", units=want)
                kind = lib.refusal_kind(set_units)
                time.sleep(1)
                probe.note("set_units", {"from": was, "to": want,
                                         "accepted": set_units.get("accepted"),
                                         "result": set_units.get("result"),
                                         "refused": set_units.get("refused"),
                                         "outcome": OUTCOME[kind]})
                probe.check(
                    "set_units is carried out, not refused",
                    set_units.get("accepted") is True,
                    contract=3, evidence={"refused": set_units.get("refused"),
                                          "outcome": OUTCOME[kind]})
                probe.check(
                    "the window shows the scale it was asked for",
                    lib.state(APP).get("units") == want,
                    contract=3, evidence={"asked_for": want, "shown": lib.state(APP).get("units")})
                # The store, not the window. This is the assertion the old code failed: the property
                # was set, the file was never written, and both looked the same from outside.
                probe.check(
                    "the choice reached ~/.config/yantrik/apps/weather/settings.json",
                    on_disk().get("fahrenheit") == (want == "fahrenheit"),
                    contract=2, evidence={"asked_for": want, "on_disk": on_disk()})

                # ── 3. Kill it and open it again ──────────────────────────────
                stop_app()
                probe.check(
                    "the file still says so with nothing running",
                    on_disk().get("fahrenheit") == (want == "fahrenheit"),
                    contract=6, evidence={"on_disk": on_disk(),
                                          "processes": lib.running(APP_BIN)})
                reopened = open_weather()
                probe.check(
                    "it opens again after being killed",
                    bool(reopened["processes"]) and reopened["surface_up"],
                    contract=6, evidence=reopened)
                after_restart = lib.state(APP)
                probe.check(
                    "the scale chosen before the restart is the scale shown after it",
                    after_restart.get("units") == want,
                    contract=6, evidence={"asked_for": want, "shown": after_restart.get("units"),
                                          "on_disk": on_disk()})

                # ── 4. A place that does not exist ────────────────────────────
                # A reading is fetched on a thread at every launch, and when it has to go straight to
                # Open-Meteo because the service is down it says so in the notice when it lands. That
                # is true and it is not what the check below is about: a reading arriving a moment
                # after the refusal would write over the refusal's line, and the check would fail for
                # a reason that has nothing to do with the lookup. So the in-flight one is waited for
                # first, and the notice is read the instant the refusal comes back.
                in_flight = lib.wait_until(
                    lambda: str(lib.state(APP).get("notice") or "").strip(),
                    timeout=10,
                    what="the reading that was in flight at launch to land and write its own line")
                probe.note("the_notice_before_the_lookup", in_flight.evidence())

                before_add = saved_places()
                nowhere = lib.act(APP, "add_location", name=NOWHERE)
                notice = str(lib.state(APP).get("notice") or "")
                nowhere_kind = lib.refusal_kind(nowhere)
                time.sleep(1)
                after_add = saved_places()
                refusal = str(nowhere.get("refused") or "")
                evidence = {"accepted": nowhere.get("accepted"), "result": nowhere.get("result"),
                            "refused": refusal, "refusal_kind": nowhere_kind,
                            "outcome": OUTCOME[nowhere_kind], "notice": notice,
                            "places_before": len(before_add), "places_after": len(after_add)}
                probe.note("add_a_place_that_is_not_one", evidence)

                probe.check(
                    "a name that cannot be found is refused, never answered with success",
                    nowhere.get("accepted") is not True,
                    contract=3, evidence=evidence)
                probe.check(
                    "the refusal arrives in words the caller can read, not \"1\"",
                    bool(refusal) and refusal not in ("1", "0"),
                    contract=4, evidence=evidence)
                probe.check(
                    "nothing was stored for a place that was not found",
                    len(after_add) == len(before_add)
                    and not any(NOWHERE.lower() in str(p.get("name", "")).lower()
                                for p in after_add),
                    contract=2, evidence={"places": after_add})

                if nowhere_kind == "policy":
                    # The ceiling answered, so the app was never asked to look anything up. The two
                    # checks below are about the app's own words; recording the ceiling's sentence as
                    # the app's answer is the fault this branch exists to prevent.
                    not_exercised += [
                        "the refusal is the app's own and says what went wrong with the lookup",
                        "the person at the window is told as well, in describe.notice",
                    ]
                    probe.note("lookup_not_exercised", {
                        "statement": "The lookup was NOT EXERCISED: the machine's ceiling refused "
                                     "`add_location` before the app saw it. A green result here is "
                                     "not coverage of what the app says when a place is not found.",
                        "the_refusal_in_full": refusal,
                        "checks_not_exercised": not_exercised,
                    })
                else:
                    # Two sentences are possible and both are the app's: the geocoder answered and
                    # knows of no such place, or it could not be reached at all. They are not the
                    # same thing and the app is required to say which.
                    names_it = NOWHERE.lower() in refusal.lower()
                    unreachable = "could not be reached" in refusal.lower()
                    probe.check(
                        "the refusal is the app's own and says what went wrong with the lookup: the "
                        "place was not found, or the geocoder could not be reached",
                        nowhere_kind == "app" and (names_it or unreachable),
                        contract=4, evidence={"refused": refusal, "names_the_place": names_it,
                                              "says_the_geocoder_was_unreachable": unreachable,
                                              "outcome": OUTCOME[nowhere_kind]})
                    probe.check(
                        "the person at the window is told as well, in describe.notice",
                        NOWHERE.lower() in notice.lower(),
                        contract=4, evidence={"notice": notice, "refused": refusal})

                # ── 5. A real place, whichever way this machine's network goes ──
                #
                # Not an assertion that the lookup succeeds — this VM may have no internet. The
                # assertion is that whichever happens is reported truthfully: a success matched by a
                # new entry in the file, a failure by a refusal and by nothing added.
                before_real = saved_places()
                real = lib.act(APP, "add_location", name="Reykjavik")
                real_kind = lib.refusal_kind(real)
                time.sleep(2)
                after_real = saved_places()
                answered = real.get("result") or {}
                probe.note("add_a_real_place", {"accepted": real.get("accepted"),
                                                "result": answered,
                                                "refused": real.get("refused"),
                                                "outcome": OUTCOME[real_kind],
                                                "places_before": len(before_real),
                                                "places_after": len(after_real),
                                                "notice": lib.state(APP).get("notice")})
                if real.get("accepted") is True:
                    probe.check(
                        "a successful add reports what the geocoder resolved, not what it was asked",
                        bool(answered.get("added")) and answered.get("lat") is not None
                        and answered.get("lon") is not None,
                        contract=3, evidence=answered)
                    probe.check(
                        "and the place it reported is in the file",
                        any(str(answered.get("added")) == p.get("name") for p in after_real),
                        contract=2, evidence={"places": after_real})
                else:
                    probe.check(
                        "an add that did not happen stored nothing",
                        len(after_real) == len(before_real),
                        contract=2, evidence={"places": after_real})
                    probe.check(
                        "and says why, to the caller and on screen",
                        bool(real.get("refused"))
                        and bool(str(lib.state(APP).get("notice") or "").strip()),
                        contract=4, evidence={"refused": real.get("refused"),
                                              "notice": lib.state(APP).get("notice"),
                                              "outcome": OUTCOME[real_kind]})


            finally:
                # Nothing may be writing to the file while it is being put back.
                stop_app()

        # ── Put the machine back ──────────────────────────────────────────
        after_bytes = CONFIG.read_bytes() if CONFIG.exists() else None
        probe.note("config_after", {"existed": CONFIG.exists(), "prefs": on_disk()})
        probe.check(
            "the weather prefs are left exactly as they were found, byte for byte",
            not config.differences() and after_bytes == before_bytes,
            contract="leave-as-found",
            evidence={"before": config.listing(), "after": config.listing(config.after),
                      "differences": config.differences() or "none",
                      "identical_bytes": after_bytes == before_bytes})

        if was_running:
            open_weather()
        leftover = lib.running(APP_BIN)
        probe.note("processes_after", leftover)
        probe.check(
            "a weather app is running afterwards exactly if one was running before",
            bool(leftover) == was_running,
            contract="leave-as-found",
            evidence={"before": probe.notes["processes_before"], "after": leftover})


if __name__ == "__main__":
    run()
