#!/usr/bin/env python3
"""yDoc's one job: write a document and keep it.

Until today this app could not keep anything. `on_doc_save` read `doc-file-path`, an `in`
property that nothing in the process ever wrote, found it empty, logged "No file path set" and
returned; the `std::fs::write` beneath the guard had never executed once. There was no command
line and `on_doc_open` was a log line, so no path could reach the app by any route. What made it
urgent is what did work: the AI path really asks the companion and really replaces the whole
document, so the app could take your writing away and give you something else, and you could not
save the result.

So the check at the centre of this probe is the one that was impossible before: `set_content`,
then `save_as`, then read the bytes off the disk and compare them to the text that was sent. The
witness is always the file — its bytes, its size, its content after a kill — and never the
action's own answer. `describe` is checked against the file, not the other way round.

The app publishes as `documents`, which is the id the launcher routes (`dock.rs`:
`Launch::Program { id: "documents", bin: "yantrik-document-editor" }`) and the id the window sets
on `AppIdentity`. This file is named after the app so the report reads `document-editor`; the two
names are the same app and the constants below keep them apart.
"""

import os
import pathlib
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import lib  # noqa: E402

ONE_JOB = ("Write a document and keep it: what is set or appended is the Markdown on disk "
           "byte for byte, it is still there after the app is killed, and a save that cannot "
           "be made says why instead of answering success.")

PROBE = "document-editor"
# The control surface's id, which is also what `open_app` is asked for.
SURFACE = "documents"
APP_BIN = "/opt/yantrik/bin/yantrik-document-editor"
# Where the unsaved draft lives between sessions (the framework's draft lane, under the app's
# state folder; the build before the framework kept it in `.../document-editor/draft.json`). The
# app writes here on its own schedule, so it is snapshotted and put back even though nothing in
# this probe writes it deliberately.
DRAFTS = pathlib.Path.home() / ".local/state/yantrik/documents"
# The old build's folder. A first start imports its `draft.json` into the lane and renames it, so
# restoring only the lane would delete the imported copy and leave the person's draft as a file
# nothing reads.
OLD_DRAFTS = pathlib.Path.home() / ".local/state/yantrik/document-editor"

FIRST = "# Conformance\n\nThe first paragraph, with a — dash and नमस्ते.\n"
SECOND = "## Appended\n\nA second section, added by the append action.\n"


def open_ydoc():
    return lib.open_app(SURFACE, expect_process=APP_BIN, window_words=("ydoc",), timeout=45)


def read(path):
    """The file's own bytes, decoded strictly. The witness for every claim below."""
    try:
        return path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        return "<<unreadable: %s>>" % exc


with lib.Probe(PROBE, ONE_JOB) as probe:
    probe.note("processes_before", lib.running(APP_BIN))
    probe.note("windows_before", lib.toplevels())

    # Resolved, because the app canonicalises the folder it writes into before it reports the
    # path back. Comparing an unresolved `/tmp/...` against a resolved one would fail on the
    # symlink and say nothing about the app.
    work = pathlib.Path(tempfile.mkdtemp(prefix="yantrik-ydoc-probe-")).resolve()
    doc_path = work / "conformance.md"
    other_path = work / "handover.md"
    other_text = "# Handover\n\nOpened by a second launch.\n"
    other_path.write_text(other_text, encoding="utf-8")

    drafts = lib.preserved(DRAFTS)
    old_drafts = lib.preserved(OLD_DRAFTS)
    try:
        with drafts, old_drafts:
            probe.note("drafts_before", drafts.listing())

            # A cold app, so nothing below is inherited from a window somebody left open.
            lib.kill_app(APP_BIN)

            # ── 1. It opens ──────────────────────────────────────────────────
            opened = open_ydoc()
            probe.check(
                "it opens: a process exists and the compositor has its window",
                bool(opened["processes"]) and bool(opened["windows"]) and opened["surface_up"],
                contract=1, evidence=opened)
            probe.check(
                "a launch that worked adds nothing to the shell's failed_launches",
                opened["new_failed_launches_for_this_app"] == [],
                contract=1, evidence={"added_by_this_launch": opened["new_failed_launches"]})

            # ── 2. The one job: what is written is what lands on disk ────────
            #
            # This is the check the old app could not have passed by any route.
            wrote = lib.act(SURFACE, "set_content", text=FIRST)
            saved = lib.act(SURFACE, "save_as", path=str(doc_path))
            on_disk = read(doc_path)
            probe.check(
                "set_content then save_as puts exactly that text in the file on disk",
                doc_path.exists() and on_disk == FIRST,
                contract=2, evidence={"path": str(doc_path), "expected": FIRST,
                                      "on_disk": on_disk, "exists": doc_path.exists(),
                                      "set_content": wrote.get("result"),
                                      "set_content_refused": wrote.get("refused"),
                                      "save_as": saved.get("result"),
                                      "save_as_refused": saved.get("refused")})

            # ── 3. The save reports what it observed, not what it was asked ──
            result = saved.get("result") or {}
            size_on_disk = doc_path.stat().st_size if doc_path.exists() else None
            probe.check(
                "save_as answers with the file's own size, re-read after the write",
                result.get("bytes") == size_on_disk and result.get("saved") == str(doc_path),
                contract=3, evidence={"answered_bytes": result.get("bytes"),
                                      "size_on_disk": size_on_disk,
                                      "answered_path": result.get("saved"),
                                      "modified_unix": result.get("modified_unix")})

            view = lib.state(SURFACE)
            probe.check(
                "describe agrees with the file: the path it names holds the words it counts",
                view.get("path") == str(doc_path) and view.get("dirty") is False
                and view.get("words") == len(on_disk.split()),
                contract=2, evidence={"describe_path": view.get("path"),
                                      "describe_dirty": view.get("dirty"),
                                      "describe_words": view.get("words"),
                                      "words_in_the_file": len(on_disk.split()),
                                      "format": view.get("format")})
            probe.check(
                "the outline it publishes is the headings the file actually has",
                [h.get("title") for h in view.get("outline") or []] == ["Conformance"],
                contract=2, evidence={"outline": view.get("outline"),
                                      "hash_lines": [l for l in on_disk.splitlines()
                                                     if l.startswith("#")]})

            # ── append, and a plain save onto the file it came from ──────────
            added = lib.act(SURFACE, "append", text=SECOND)
            second_save = lib.act(SURFACE, "save")
            after_append = read(doc_path)
            probe.check(
                "append adds to the document and save writes it to the file it came from",
                after_append == FIRST + SECOND,
                contract=2, evidence={"expected": FIRST + SECOND, "on_disk": after_append,
                                      "append": added.get("result"),
                                      "append_refused": added.get("refused"),
                                      "save": second_save.get("result"),
                                      "save_refused": second_save.get("refused")})

            # ── 6. It survives a restart, and 5. it takes a path from outside ─
            killed = lib.kill_app(APP_BIN)
            survived = read(doc_path)
            lib.spawn([APP_BIN, str(doc_path)])
            reopened = lib.wait_until(
                lambda: lib.surface_up(SURFACE) or None,
                timeout=45, what="yDoc launched with a path on the command line to answer")
            restored = lib.state(SURFACE)
            probe.check(
                "the document is on disk after the app is killed",
                survived == FIRST + SECOND,
                contract=6, evidence={"killed": killed, "on_disk": survived})
            probe.check(
                "a path on the command line opens that document in a fresh window",
                bool(reopened) and restored.get("path") == str(doc_path)
                and restored.get("content") == survived,
                contract=5, evidence=reopened.evidence(
                    describe_path=restored.get("path"),
                    content_matches_file=restored.get("content") == survived,
                    chars_shown=restored.get("content_chars_shown"),
                    chars_total=restored.get("content_chars_total"),
                    processes=lib.running(APP_BIN)))

            # ── 5 again: a second launch hands over instead of opening twice ──
            before_handover = lib.running(APP_BIN)
            handover = lib.run_and_wait([APP_BIN, str(other_path)], timeout=30)
            handed = lib.wait_until(
                lambda: lib.state(SURFACE).get("path") == str(other_path) or None,
                timeout=20, what="the running window to take the handed-over file")
            after_handover = lib.running(APP_BIN)
            probe.check(
                "a second launch hands its file to the running window and does not open another",
                bool(handed) and len(after_handover) == len(before_handover),
                contract=5, evidence=handed.evidence(
                    second_launch_exit=handover.get("exit"),
                    stderr=handover.get("stderr"),
                    processes_before=before_handover, processes_after=after_handover,
                    describe_path=lib.state(SURFACE).get("path")))

            # ── 3/4. A save with nowhere to go is refused, in words ──────────
            #
            # THE bug, asserted directly. The old app answered this situation with a log line
            # inside its own process and nothing at all outside it.
            fresh = lib.act(SURFACE, "new")
            refused = lib.act(SURFACE, "save")
            notice = lib.state(SURFACE).get("notice") or ""
            files_now = sorted(p.name for p in work.iterdir())
            probe.check(
                "new gives an empty document with no file",
                fresh.get("accepted") is True and lib.state(SURFACE).get("path") is None,
                contract=3, evidence={"new": fresh.get("result"),
                                      "refused": fresh.get("refused"),
                                      "describe_path": lib.state(SURFACE).get("path")})
            probe.check(
                "save on a document with no file is refused with a readable reason, not success",
                lib.refusal_kind(refused) == "app"
                and "no file yet" in str(refused.get("refused") or "").lower(),
                contract=3, evidence={"accepted": refused.get("accepted"),
                                      "refusal": refused.get("refused"),
                                      "result": refused.get("result")})
            probe.check(
                "and the same reason is in describe.notice, not only in the reply",
                "no file yet" in notice.lower(),
                contract=4, evidence={"notice": notice,
                                      "refusal": refused.get("refused")})
            probe.check(
                "nothing was written while it refused",
                files_now == ["conformance.md", "handover.md"],
                contract=4, evidence={"files": files_now, "work_dir": str(work)})

            # ── 4. A file changed underneath is reported, not clobbered ──────
            lib.act(SURFACE, "open", path=str(doc_path))
            lib.act(SURFACE, "set_content", text="# Mine\n\nThe draft in the window.\n")
            outside = "# Theirs\n\nWritten by somebody else while the window was open.\n"
            doc_path.write_text(outside, encoding="utf-8")
            conflicted = lib.act(SURFACE, "save")
            conflict_notice = lib.state(SURFACE).get("notice") or ""
            still_theirs = read(doc_path)
            probe.check(
                "a file changed on disk since it was opened is not overwritten, and is said to be",
                lib.refusal_kind(conflicted) == "app"
                and "changed on disk" in str(conflicted.get("refused") or "").lower()
                and still_theirs == outside,
                contract=4, evidence={"refusal": conflicted.get("refused"),
                                      "accepted": conflicted.get("accepted"),
                                      "on_disk_after": still_theirs,
                                      "written_outside": outside})
            probe.check(
                "the conflict is in describe.notice as well as in the refusal",
                "changed on disk" in conflict_notice.lower(),
                contract=4, evidence={"notice": conflict_notice})
            probe.check(
                "the draft in the window is still the draft, and still says it is unsaved",
                lib.state(SURFACE).get("dirty") is True,
                contract=2, evidence={"dirty": lib.state(SURFACE).get("dirty"),
                                      "content": lib.state(SURFACE).get("content"),
                                      "summary": lib.describe(SURFACE).get("summary")})

            # ── The write itself ─────────────────────────────────────────────
            lib.kill_app(APP_BIN)
            leftovers = sorted(p.name for p in work.glob(".yantrik-ydoc-*"))
            probe.check(
                "no half-written temporary file is left beside the document",
                leftovers == [],
                contract=6, evidence={"temp_files": leftovers,
                                      "work_dir_listing": sorted(p.name for p in work.iterdir())})

    finally:
        # ── Put the machine back ─────────────────────────────────────────────
        #
        # `preserved` has already restored the draft directory; what is left is this probe's own
        # scratch tree. Every path this probe named was inside it, so nothing in ~/Documents was
        # touched.
        lib.kill_app(APP_BIN)
        shutil.rmtree(work, ignore_errors=True)

    probe.note("drafts_after", drafts.listing(drafts.after))
    probe.check(
        "the draft directory is left as it was found",
        not drafts.differences(),
        contract="leave-as-found",
        evidence={"before": drafts.listing(), "after": drafts.listing(drafts.after),
                  "differences": drafts.differences() or "none"})
    probe.check(
        "the probe's scratch directory is gone",
        not work.exists(),
        contract="leave-as-found", evidence={"work_dir": str(work)})

    leftover = lib.running(APP_BIN)
    probe.note("processes_after", leftover)
    probe.note("windows_after", lib.toplevels())
    probe.check(
        "no yDoc process is left running that was not running before",
        leftover == probe.notes["processes_before"],
        contract="leave-as-found",
        evidence={"before": probe.notes["processes_before"], "after": leftover})
