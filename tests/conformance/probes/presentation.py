#!/usr/bin/env python3
"""yPresent's one job: keep a deck of slides and show them in order.

The fault this is mostly about is one no screenshot could have found. `on_next_slide` and
`on_prev_slide` moved `current-slide-index` and did nothing else — no commit of what was on the
canvas, no reload of the slide moved to — so the canvas went on holding the text of the slide
you had just been typing into, and the next thing that committed it wrote that text into the
slide you had moved to. Typing, pressing Next, and clicking a thumbnail destroyed a slide. The
window looked correct at every step, and the six handlers that were written correctly are what
did the overwriting.

So the middle of this probe is that sequence, driven over the surface: two slides with distinct
text, `next`, `go_to`, and then a reading of what the second slide actually says. It fails on
the old code and passes on the new one.

The rest is the contract. Save and Load were log lines, so a deck existed only while the window
was open: the deck is written to a path this probe chose, checked as a file this probe parses
itself, and then the app is killed and started again with that path on its command line —
contract points 2, 5 and 6 in one move, verified against the bytes on disk and never against the
action's own answer.

`delete_slide` is graded `sensitive` because its whole effect is that a slide someone wrote is
gone. The machine's ceiling is the user's setting; when it stands in front of this action the
app's delete code did not run and was not measured, and the checks that were about the app are
recorded as NOT EXERCISED rather than passed on the ceiling's sentence. This probe does not
raise the ceiling: a green run bought that way would be worth less than an honest omission.

Nothing is left behind. The deck goes in a temp directory of this probe's own, and
`~/Documents/Presentations` and the app's state folder under `~/.local/state/yantrik/presentation`
(the framework's `drafts/` lane, and the old build's `recovery.json` with the `.imported` copy a
first start makes of it) are both snapshotted on the way in and restored on the way out — those
are the two places the app writes to when nobody has named a path.
"""

import json
import os
import pathlib
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import lib  # noqa: E402

ONE_JOB = ("Keep a deck of slides and show them in order: a slide keeps the text it was given "
           "however you move through the deck, the deck is a file, and the file is still that "
           "deck after the app has been killed.")

APP = "presentation"
APP_BIN = "/opt/yantrik/bin/yantrik-presentation"

# The two places the app writes when nobody names a path. Both are restored at the end.
DECKS = pathlib.Path.home() / "Documents/Presentations"
STATE_HOME = pathlib.Path(os.environ.get("XDG_STATE_HOME") or (pathlib.Path.home() / ".local/state"))
RECOVERY_DIR = STATE_HOME / "yantrik/presentation"

# The distinct text. Nothing here is a substring of anything else here, so "slide 2 says slide
# 1's words" cannot be read as a coincidence.
ALPHA_TITLE = "Alpha the first"
ALPHA_BODY = "aaa body of the first added slide"
BETA_TITLE = "Beta the second"
BETA_BODY = "bbb body of the second added slide"

OUTCOME = {
    "policy": "refused by policy — the machine's ceiling turned it away before the app saw it",
    "app": "refused by the app",
    None: "answered by the app",
}

WHY_NOT_EXERCISED = (
    "`presentation.delete_slide` is graded `sensitive`, above this machine's ceiling, so the "
    "control surface refused on the grade alone, before dispatch. The app's delete code did not "
    "run and was not measured. The ceiling is the user's setting, in `tool_permission` in "
    "~/.config/yantrik/settings.yaml; raising it is their decision, not this probe's."
)

not_exercised = []
ceiling_refusal = None


def outline():
    """The slide titles the app says it holds, off `describe`."""
    return lib.state(APP).get("outline") or []


def read_deck(path):
    """The deck file as this probe parses it. The witness for points 2 and 6.

    Deliberately not the app's answer and not the app's parser: a `.ydeck` is JSON with a
    version in front of it, and if that stops being readable by anything but this app then
    "diffable, and a mind can read it" was not true.
    """
    try:
        return json.loads(pathlib.Path(path).read_text())
    except (OSError, ValueError) as exc:
        return {"unreadable": "%s: %s" % (type(exc).__name__, exc)}


def titles_in(document):
    return [s.get("title") for s in document.get("slides") or [] if isinstance(s, dict)]


def open_presentation():
    return lib.open_app(APP, expect_process=APP_BIN, window_words=("ypresent",), timeout=45)


with lib.Probe(APP, ONE_JOB) as probe:
    probe.note("processes_before", lib.running(APP_BIN))
    probe.note("windows_before", lib.toplevels())

    work_dir = pathlib.Path(tempfile.mkdtemp(prefix="yantrik-deck-probe-"))
    deck_path = work_dir / "conformance.ydeck"

    decks = lib.preserved(DECKS)
    drafts = lib.preserved(RECOVERY_DIR)
    try:
        with decks, drafts:
            probe.note("decks_before", decks.listing())
            probe.note("drafts_before", drafts.listing())

            # A cold app, so nothing below is inherited from a window that was already open.
            lib.kill_app(APP_BIN)

            # ── 1. It opens ──────────────────────────────────────────────────
            opened = open_presentation()
            probe.check(
                "it opens: a process exists, the compositor has its window, and the surface "
                "answers",
                bool(opened["processes"]) and bool(opened["windows"]) and opened["surface_up"],
                contract=1, evidence=opened)
            probe.check(
                "this launch added nothing to the shell's failed_launches",
                not opened["new_failed_launches_for_this_app"],
                contract=1, evidence={"added_by_this_launch": opened["new_failed_launches"]})

            published = lib.actions(APP)
            probe.note("actions", published)
            probe.check(
                "the surface publishes the verbs a deck needs",
                {"open", "save", "save_as", "new_deck", "add_slide", "set_slide", "delete_slide",
                 "move_slide", "go_to", "next", "previous", "present",
                 "export_markdown"}.issubset(set(published)),
                contract=3, evidence={"published": published})

            # ── 2. It does its one job ───────────────────────────────────────
            started = lib.act(APP, "new_deck", title="Conformance deck")
            probe.check(
                "a new deck starts with one slide and says what it is called",
                started.get("accepted") is True
                and (started.get("result") or {}).get("deck_title") == "Conformance deck"
                and (started.get("result") or {}).get("slides") == 1,
                contract=2, evidence={"result": started.get("result"),
                                      "refused": started.get("refused")})

            first = lib.act(APP, "add_slide", title=ALPHA_TITLE, body=ALPHA_BODY)
            second = lib.act(APP, "add_slide", title=BETA_TITLE, body=BETA_BODY)
            after_adds = outline()
            probe.check(
                "two added slides are both in the deck, in the order they were added",
                after_adds[-2:] == [ALPHA_TITLE, BETA_TITLE],
                contract=2, evidence={"outline": after_adds,
                                      "first": first.get("result"),
                                      "second": second.get("result"),
                                      "refused": first.get("refused") or second.get("refused")})
            if len(after_adds) < 3:
                raise SystemExit("the deck did not take the two slides; nothing below is testable")

            alpha_at = after_adds.index(ALPHA_TITLE)
            beta_at = after_adds.index(BETA_TITLE)

            # ── The bug ──────────────────────────────────────────────────────
            #
            # Stand on the first added slide, press Next, and come back. Under the old code the
            # canvas still held slide N's text when the index said N+1, and coming back
            # committed it into N+1 — so the check is what the SECOND slide says afterwards,
            # not what the action answered.
            lib.act(APP, "go_to", index=alpha_at)
            stepped = lib.act(APP, "next")
            lib.act(APP, "go_to", index=alpha_at)

            after_walk = outline()
            landed = lib.act(APP, "go_to", index=beta_at)
            beta_now = lib.state(APP).get("current") or {}

            probe.check(
                "walking to the next slide and back does not write one slide over the next",
                after_walk == after_adds
                and beta_now.get("title") == BETA_TITLE
                and beta_now.get("body") == BETA_BODY,
                contract=2,
                evidence={"outline_before_the_walk": after_adds,
                          "outline_after_the_walk": after_walk,
                          "slide_2_now": beta_now,
                          "slide_2_should_be": {"title": BETA_TITLE, "body": BETA_BODY},
                          "next_answered": stepped.get("result"),
                          "go_to_answered": landed.get("result"),
                          "note": "the old Next moved the index without committing the canvas "
                                  "or reloading it, so the following commit wrote slide N over "
                                  "slide N+1"})
            lib.act(APP, "go_to", index=alpha_at)
            alpha_now = lib.state(APP).get("current") or {}
            probe.check(
                "the first slide still holds its own text too",
                alpha_now.get("title") == ALPHA_TITLE and alpha_now.get("body") == ALPHA_BODY,
                contract=2, evidence={"slide_1_now": alpha_now,
                                      "slide_1_should_be": {"title": ALPHA_TITLE,
                                                            "body": ALPHA_BODY}})

            # `next` and `go_to` have to report where the deck actually is, not where they were
            # asked to put it.
            probe.check(
                "navigation answers with the slide it landed on",
                (stepped.get("result") or {}).get("index") == beta_at
                and (stepped.get("result") or {}).get("title") == BETA_TITLE,
                contract=3, evidence={"next": stepped.get("result"),
                                      "expected_index": beta_at})

            # ── 2 again: the store agrees ────────────────────────────────────
            saved = lib.act(APP, "save_as", path=str(deck_path))
            document = read_deck(deck_path)
            probe.check(
                "the deck it says it saved is a file on disk holding both slides",
                deck_path.exists()
                and ALPHA_TITLE in titles_in(document)
                and BETA_TITLE in titles_in(document),
                contract=2, evidence={"path": str(deck_path), "exists": deck_path.exists(),
                                      "titles_on_disk": titles_in(document),
                                      "save_answered": saved.get("result"),
                                      "refused": saved.get("refused")})
            probe.check(
                "the file says which version of the format it is",
                isinstance(document.get("version"), int),
                contract=6, evidence={"version": document.get("version"),
                                      "keys": sorted(k for k in document if k != "slides")})
            probe.check(
                "the bodies on disk are the bodies that were typed, slide by slide",
                [s.get("body") for s in document.get("slides") or []][-2:]
                == [ALPHA_BODY, BETA_BODY],
                contract=2, evidence={"bodies_on_disk":
                                      [s.get("body") for s in document.get("slides") or []]})
            probe.check(
                "a deck that was just written is not still reported as unsaved",
                lib.state(APP).get("dirty") is False
                and lib.state(APP).get("file") == str(deck_path),
                contract=3, evidence={"dirty": lib.state(APP).get("dirty"),
                                      "file": lib.state(APP).get("file"),
                                      "save_answered": saved.get("result")})
            probe.check(
                "saving leaves no half-written file beside the deck",
                sorted(p.name for p in work_dir.iterdir()) == ["conformance.ydeck"],
                contract=6, evidence={"work_dir": sorted(p.name for p in work_dir.iterdir())})

            on_disk_before_kill = read_deck(deck_path)

            # ── 5 and 6. Killed, and started again with the path ─────────────
            killed = lib.kill_app(APP_BIN)
            probe.check(
                "the deck file outlives the process",
                read_deck(deck_path) == on_disk_before_kill,
                contract=6, evidence={"killed": killed, "path": str(deck_path)})

            lib.spawn([APP_BIN, str(deck_path)])
            lib.wait_for(lambda: lib.surface_up(APP), timeout=45)

            def opened_the_deck():
                view = lib.state(APP)
                return view if view.get("file") == str(deck_path) else None

            back = lib.wait_until(
                opened_the_deck, timeout=20,
                what="the reopened window to report the deck it was handed")
            restored = lib.state(APP)
            probe.check(
                "started again with a path on its command line, it opens that deck",
                bool(back) and bool(lib.running(APP_BIN)) and lib.has_window("ypresent")
                and restored.get("file") == str(deck_path),
                contract=5, evidence=back.evidence(
                    processes=lib.running(APP_BIN), windows=lib.toplevels(),
                    file=restored.get("file"), summary=restored.get("summary")))

            titles_on_disk = titles_in(on_disk_before_kill)
            probe.check(
                "every slide is back, with its own text, in its own order",
                bool(titles_on_disk) and restored.get("outline") == titles_on_disk,
                contract=6, evidence={"outline_now": restored.get("outline"),
                                      "titles_on_disk": titles_on_disk,
                                      "slides": restored.get("slides")})
            probe.check(
                "a deck read off disk does not open claiming to have unsaved changes",
                restored.get("dirty") is False and restored.get("recovered") is False,
                contract=3, evidence={"dirty": restored.get("dirty"),
                                      "recovered": restored.get("recovered"),
                                      "notice": restored.get("notice")})

            # ── 3 and 4. A refusal names its reason, on screen as well ───────
            #
            # A file that is not a deck. The deck on screen must be exactly what it was: the
            # app loads the file before it displaces anything, which is what "never half-load"
            # means from the outside.
            before_bad_open = lib.state(APP)
            bad = lib.act(APP, "open", path="/etc/hostname")
            after_bad_open = lib.state(APP)
            bad_kind = lib.refusal_kind(bad)
            probe.check(
                "a file that is not a deck is refused in words, not accepted and dropped",
                bad.get("accepted") is not True and bool(bad.get("refused"))
                and bad.get("refused") not in ("1", "0"),
                contract=4, evidence={"refusal": bad.get("refused"), "kind": bad_kind,
                                      "outcome": OUTCOME[bad_kind]})
            probe.check(
                "the deck that was open is untouched by a load that failed",
                after_bad_open.get("outline") == before_bad_open.get("outline")
                and after_bad_open.get("file") == before_bad_open.get("file"),
                contract=2, evidence={"before": before_bad_open.get("outline"),
                                      "after": after_bad_open.get("outline"),
                                      "file": after_bad_open.get("file")})
            if bad_kind == "app":
                probe.check(
                    "the same failure is on screen, not only in the caller's error",
                    bool((after_bad_open.get("notice") or "").strip()),
                    contract=4, evidence={"notice": after_bad_open.get("notice"),
                                          "refusal": bad.get("refused")})
            else:
                not_exercised.append(
                    "the same failure is on screen, not only in the caller's error (open)")

            missing = lib.act(APP, "go_to", index=999)
            missing_kind = lib.refusal_kind(missing)
            missing_evidence = {"refusal": missing.get("refused"), "kind": missing_kind,
                                "outcome": OUTCOME[missing_kind],
                                "slides": lib.state(APP).get("slides")}
            probe.check(
                "asking for a slide that is not there is refused, never reported as a move",
                missing.get("accepted") is not True,
                contract=3, evidence=missing_evidence)
            if missing_kind == "app":
                probe.check(
                    "the refusal names the slide asked for and the deck it looked in",
                    "slide 1000" in str(missing.get("refused") or "")
                    and "deck of" in str(missing.get("refused") or ""),
                    contract=3, evidence=missing_evidence)
            else:
                not_exercised.append(
                    "the refusal names the slide asked for and the deck it looked in")

            # ── 9. The graded one ────────────────────────────────────────────
            before_delete = lib.state(APP)
            deck_now = before_delete.get("outline") or []
            target = deck_now.index(ALPHA_TITLE) if ALPHA_TITLE in deck_now else 0
            deleted = lib.act(APP, "delete_slide", index=target)
            kind = lib.refusal_kind(deleted)
            after_delete = lib.state(APP)
            evidence = {"index": target, "accepted": deleted.get("accepted"),
                        "result": deleted.get("result"), "refused": deleted.get("refused"),
                        "refusal_kind": kind, "outcome": OUTCOME[kind],
                        "outline_before": before_delete.get("outline"),
                        "outline_after": after_delete.get("outline")}
            probe.note("delete_slide", evidence)

            if kind == "policy":
                ceiling_refusal = deleted.get("refused")
                probe.note("delete_path_exercised", False)
                probe.check(
                    "a sensitive action refused by the ceiling removes nothing",
                    after_delete.get("outline") == before_delete.get("outline"),
                    contract=9, evidence=evidence)
                probe.check(
                    "the refusal is in words the caller can read, not \"1\"",
                    bool(deleted.get("refused")) and deleted.get("refused") not in ("1", "0"),
                    contract=4, evidence=evidence)
                not_exercised += [
                    "the slide the app said it deleted is gone from the deck",
                    "the answer names the slide that was removed and what is left",
                ]
            elif kind == "app":
                probe.note("delete_path_exercised", True)
                probe.check(
                    "an app that says it did not delete the slide has not deleted it",
                    after_delete.get("outline") == before_delete.get("outline"),
                    contract=3, evidence=evidence)
                probe.check(
                    "the refusal is in words the caller can read, not \"1\"",
                    bool(deleted.get("refused")) and deleted.get("refused") not in ("1", "0"),
                    contract=4, evidence=evidence)
            else:
                probe.note("delete_path_exercised", True)
                answer = deleted.get("result") or {}
                probe.check(
                    "the slide the app said it deleted is gone from the deck",
                    ALPHA_TITLE not in (after_delete.get("outline") or [])
                    and after_delete.get("slides") == before_delete.get("slides") - 1,
                    contract=2, evidence=evidence)
                probe.check(
                    "the answer names the slide that was removed and what is left",
                    answer.get("deleted") == ALPHA_TITLE
                    and answer.get("slides") == after_delete.get("slides"),
                    contract=3, evidence=answer)
                probe.check(
                    "a delete that has not been saved is reported as unsaved",
                    after_delete.get("dirty") is True,
                    contract=3, evidence={"dirty": after_delete.get("dirty")})
                # And it must not reach the file until someone saves.
                probe.check(
                    "the deleted slide is still on disk until the deck is saved",
                    ALPHA_TITLE in titles_in(read_deck(deck_path)),
                    contract=2, evidence={"titles_on_disk": titles_in(read_deck(deck_path))})

            # ── Put the machine back ─────────────────────────────────────────
            #
            # Killed rather than closed: a close would commit the canvas and write a recovery
            # draft for the unsaved delete above, which is correct behaviour and not something
            # to leave on a shared machine.
            lib.kill_app(APP_BIN)

    finally:
        lib.kill_app(APP_BIN)
        shutil.rmtree(work_dir, ignore_errors=True)

    if not_exercised:
        probe.note("not_exercised", not_exercised)
        probe.note("why_not_exercised", WHY_NOT_EXERCISED)
        probe.note("ceiling_refusal", ceiling_refusal)

    probe.note("decks_after", decks.listing(decks.after))
    probe.note("drafts_after", drafts.listing(drafts.after))
    probe.check(
        "the default deck folder is left as it was found",
        not decks.differences(),
        contract="leave-as-found",
        evidence={"path": str(DECKS), "before": decks.listing(),
                  "after": decks.listing(decks.after),
                  "differences": decks.differences() or "none"})
    probe.check(
        "no recovery draft is left behind",
        not drafts.differences(),
        contract="leave-as-found",
        evidence={"path": str(RECOVERY_DIR), "before": drafts.listing(),
                  "after": drafts.listing(drafts.after),
                  "differences": drafts.differences() or "none"})
    probe.check(
        "the probe's scratch directory is gone",
        not work_dir.exists(),
        contract="leave-as-found", evidence={"work_dir": str(work_dir)})

    leftover = lib.running(APP_BIN)
    probe.note("processes_after", leftover)
    probe.note("windows_after", lib.toplevels())
    probe.check(
        "no yPresent process is left running that was not running before",
        leftover == probe.notes["processes_before"],
        contract="leave-as-found",
        evidence={"before": probe.notes["processes_before"], "after": leftover})
