# Notes, the framework's acceptance gate (21 Oct 2026, revision 2)

Step 8 of [the app framework](app-framework-2026-10-21.md): "two document contexts, save failure, an interrupted migration, a conflicting outside edit, close cancelled, recovery after a forced kill. All of these pass before any other app moves." Weather proved what a dashboard needs. Notes is the first app that edits documents, so this is where the framework must grow.

Revision 1 answered each gate item with an app patch plus two framework primitives (watched paths, secondary windows). The architect's critique (GPT-5.4, §8) rejected it: the gate asks for **documents**, and windows are only how documents are shown. This revision puts a document model in the framework and builds the rest on it. Decisions marked ★ are Y's under the 20 Oct delegation; Pranab may overturn any of them.

## 1. What Notes is today

`apps/notes`: a Markdown vault, one `.md` file (plus an optional `.meta`) per note, under `~/.local/share/yantrik/notes`. That is exactly the framework's `Paths::data_dir` for the id `notes`, so the vault does not move. `store.rs` (344 lines) is careful already: bounded reads (`O_NOFOLLOW`, size limits), atomic writes (a temp file, then a directory fsync), a per-note **baseline** (the text as last read), a **write-ahead journal** (`.recovery-v2/pending.json`) around every save, a trash folder, and a refusal to save over a file that no longer matches its baseline. `main.rs` (1,905 lines) is one window, one current note, a filesystem worker thread, a 750 ms autosave, undo and redo, import and export, and the control surface. The vault has been format v2 since its first commit and has never been migrated.

## 2. The addition: documents, in the framework

★ The framework gains `yantrik_app::documents`, the state of every document an app has open, independent of windows. It is built for Notes and shaped for every editor after it (Text Editor, Document Editor, Spreadsheet, Presentation).

```rust
pub struct DocumentId(u64);              // stable for the document's life in this run

pub struct Document<C> {                 // owned by the app's model, in a Documents<C> table
    pub id: DocumentId,
    pub source: Source,                  // the file it came from, or New
    pub content: C,                      // the app's own type (Notes: text + meta)
    pub baseline: Fingerprint,           // the file as last read or written by us (length + hash)
    pub save: SaveState,                 // Idle | Saving{seq} | Failed{seq, error, durability}
    pub outside: OutsideState,           // Clean | Changed{seen} | Deleted{seen} | Conflict{theirs}
    pub draft: DraftState,               // None | Kept{seq} | Failed{error}
    pub seq: u64,                        // bumped on every edit; orders saves, drafts and outcomes
}
```

**Rules the module enforces, so no app re-invents them:**
- **One save in flight per document.** An edit during a save marks the document dirty again and queues one follow-up save at most, never a pile. A failed save does not retry by itself: the next edit, or the person, retries it, so a read-only disk does not spam a notice every 750 ms.
- **Durability is said honestly.** `Failed { durability }` is `NotWritten` (the original file is untouched), `ContentOnly` (the text landed and the metadata did not; the journal recovers it) or `Unknown` (an fsync failed after the rename). Every one of them leaves the document dirty, and `describe` reports which one it is.
- **Outside changes are classified, not guessed.** The watch thread fingerprints each open document's files. A fingerprint equal to the baseline, or to a write this process just committed, is not a change (self-write suppression by fingerprint, as the settings watch does). Anything else is `Changed`, and `Deleted` if the file is gone. If the document is also dirty, it is `Conflict` and carries the file as it now is. A change seen while a save is in flight is re-checked when the save's outcome lands, so a save that won, a save that lost, and a change that arrived after our rename are three different results.
- **Drafts are per document, ordered by `seq`.** A draft is kept on every edit, debounced, and dropped only when a save of the same or a later `seq` has committed. If the draft cannot be kept, the person is told, because recovery is then not protected. At start the precedence is: a draft newer than both the file and the journal wins and is offered back; otherwise the journal (an interrupted save); otherwise the file.
- **Close is a document flow.** Closing a view only asks about a document if it is the last view on it and the document is dirty: *Save / Don't save / Cancel*. Quitting asks once over every dirty document. While a close waits on a save, edits continue and the save that closes is the latest `seq`. A failed save keeps the view open and says why.

**Views and windows.** A view shows one document: its selection, scroll, undo stack and any dialog belong to the view, and the content belongs to the document. Two views on one note share its content, dirty state, conflict and draft, and each keeps its own undo history. Undo applies only to that view's own edits, and if the other view has changed the text since, its entries are dropped, not misapplied. A window holds views. The framework adds **secondary windows** (`fx.open_window`, `App::wire_window`, per-window render, per-window close through `CloseRequested`), and the app exits when its last window closes and no document refuses. No window is privileged. If Slint cannot run two windows of one component well under the compositor, the same views go in tabs, with no change to the document model.

**Watched paths** are the low-level layer under this: the watch thread polls registered paths (a `stat` first, then the hash only when the length or mtime moved), and registrations are owned by a document and dropped with it. Polling stays at 2 s, the settings watch's period. Notes watches only the open documents' files, never the whole vault.

## 3. The six gate items, on that model

| Gate item | How it is met | The tests a reviewer will believe |
|---|---|---|
| **Save failure** | `SaveState::Failed` with its durability; dirty kept; one save in flight; no self-retry; the close flow refuses to close on a failed save without the person choosing *Don't save* | Through an injectable store (`trait Disk`): the temp file cannot be created; a short write; the rename fails; the directory fsync fails after the rename (`Unknown`); the journal cannot be written (the save does not proceed); the metadata write fails after the content (`ContentOnly`, then the journal recovers it at the next start); repeated autosave ticks on a read-only vault keep at most one save in flight and one notice. On 520: a vault made read-only mid-session, then restored, then the retry succeeds |
| **Conflicting outside edit** | `OutsideState` from the watch, re-checked against save outcomes; *Keep mine as a copy* (a new note with my text, the original untouched), *Take theirs* (the file's text replaces mine; my text is kept as a recovery note first; the undo history of every view on it is cleared), *Compare* (side by side, read-only), *Not now* (keep editing; the next save is refused until it is resolved) | Change detected before the next save; with the watch disabled, the save is still refused (the store's check); deletion (the save becomes *Save as new*, never a silent re-create); a change during an in-flight save in each of the three orders; our own save does not raise a change; a metadata-only change; one note in two views, both showing the same conflict and either resolving it for both. On 520: a real outside editor changes an open note |
| **Recovery after a forced kill** | Drafts by `seq`, with the precedence above | A draft kept after the debounce; dropping the app without shutdown, then starting again, offers the text back; a draft plus an interrupted save (the journal), where the newer one wins; a save that commits removes the draft and no stale draft is offered; three notes with drafts; a draft that cannot be written is said; one note in two views gives one draft. On 520: `kill -9` while typing |
| **Close cancelled** | The close flow above | Clean close: no prompt. Dirty close: *Cancel* keeps everything and saves nothing; *Save* closes only when the save lands; a failed *Save* keeps the window and says why; *Don't save* discards the edits and the draft unless another view still has the document; a second close during the save makes no second prompt and no second save; closing one of two views on a note does not prompt; quitting with two dirty notes prompts once. On 520: the window's close button |
| **Two document contexts** | Documents, views, and secondary windows | Two windows with different notes, each with its own undo; one note in two windows, an edit in one shown in the other with dirty and conflict state shared; save and undo go to the focused view; closing one window leaves the app running; the last window closing with a dirty document prompts; a document's watches are dropped when its last view closes. On 520: two real windows (focus, typing, chrome, theme) |
| **Interrupted migration** | ★ **Not applicable to Notes, said as such.** Notes has no settings, and its vault has been v2 since the first commit, so there is nothing to migrate and no interruption to survive. Inventing a migration to tick the box would ship one nobody needs. Two things are done instead. First, groundwork: the vault gains `.vault-version`, written atomically once on a vault without it, read at every load, and a newer version is refused without touching anything, so the first real v3 has something to read. Second, the framework requirement moves to where it is real: an app-data migration helper (preserve the source until the destination commits; resume idempotently; protected recovery otherwise), accepted on its own interruption tests when the first app needs it | The marker: written once on a new vault and on an old one without it; never rewritten; a newer version refused with the vault unchanged; if the marker cannot be written, Notes opens read-only and says why; two instances creating it race to one valid file. The step-4 settings tests stay the evidence for the settings path |

## 4. Order of work

1. **8a, documents** (framework): the `documents` module, watched paths under it, and the store seam (`trait Disk`) that makes faults injectable. Tested in `examples/hello-app` with a toy text document.
2. **8b, secondary windows** (framework): the core's window table, views, `fx.open_window`, per-window close, and `TestApp` driving two windows headless.
3. **8c, Notes on the framework, behaviour unchanged**: a parity PR like Weather's. The store moves behind `trait Disk`, and the worker becomes ordered saves through `documents`. `describe` and the actions and their grades stay identical, and the store's tests and `tests.rs` move with it.
4. **8d, the gate**: every test in §3 on Notes with a real vault directory, plus 520 evidence for the five marked.

Each is its own PR with its own review. Nothing else moves onto the framework until 8d passes. The four dashboard ports (#900, #901, #903, #904) use none of this and do not wait.

## 5. Critique 1 (GPT-5.4 as the framework's architect), condensed

> As a framework architect, I would reject this row as too window-centric. The gate should force introduction of a document/context abstraction, even if the immediate Notes UI uses multiple windows.

Taken in this revision:
- **Documents, not windows.** A document owns content, baseline, dirty, save, outside and draft state, and views and windows only show it.
- **A save state machine:** one save in flight, coalescing, no self-retry spam, and durability stated as `NotWritten`, `ContentOnly` or `Unknown`.
- **Outside changes classified** into changed, deleted and conflict, plus three results for a change that arrives during an in-flight save.
- **Self-write suppression.**
- **Drafts ordered against the journal** by sequence number.
- **Close as a document flow:** only the last view of a dirty document asks; quitting asks once; no privileged primary window.
- **Real fault injection** for every way a save can fail.
- **Migration marked not applicable, honestly.** It is not counted as a pass. The marker is groundwork, and the framework's migration helper is accepted on its own tests.

Not taken:
- Rename tracking. Polling sees a rename as a deletion, and Notes says so: *the file was moved or deleted*. Real rename tracking would need directory scans or inotify, which revision 1's polling choice rules out.
- "Multiple app instances". Notes is single-instance through the framework's claim.
