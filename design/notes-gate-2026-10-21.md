# Notes, the framework's acceptance gate (21 Oct 2026)

Step 8 of [the app framework](app-framework-2026-10-21.md): "two document contexts, save failure, an interrupted migration, a conflicting outside edit, close cancelled, recovery after a forced kill. All of these pass before any other app moves." Weather proved what a dashboard needs; Notes is the first app that edits documents, so this is where the framework must grow or be shown sufficient. This note maps each gate item to what exists, what is missing, and the smallest addition that closes it.

Decisions marked ★ are Y's under the 20 Oct delegation; Pranab may overturn any of them.

## What Notes is today

`apps/notes`: a Markdown vault, one `.md` file (plus an optional `.meta`) per note, under `~/.local/share/yantrik/notes` — which is exactly the framework's `Paths::data_dir` for the id `notes`, so the vault does not move. `store.rs` (344 lines) is already careful: bounded reads (`O_NOFOLLOW`, size limits), atomic writes with a temp file and a directory fsync, a per-note **baseline** (the text as last read from disk), a **write-ahead journal** (`.recovery-v2/pending.json`) around every save, a trash folder, and a check before every save that the file on disk is still the baseline. `main.rs` (1,905 lines) is one window, one current note, one filesystem worker thread, a 750 ms autosave, undo/redo, import/export, and the control surface. The vault format has been v2 since its first commit; it has never been migrated.

## The six gate items

| Gate item | What already holds it | What is missing | Addition |
|---|---|---|---|
| **Save failure** | `store::atomic` fails whole or not at all; the journal survives | The failure must reach the person and the Mind, the text must stay, and a retry must be possible | App only: the save is a job; `Saved(Err)` sets the notice, keeps the note dirty, keeps the draft. Test: a vault directory made read-only mid-session |
| **Conflicting outside edit** | `store::check` refuses a save whose file no longer matches the baseline ("Your draft is protected; use Save a copy") | Nothing notices the outside edit until the next save, so the person can type for minutes over a note that changed under them | **Framework: watched paths.** The watch thread (2 s, fingerprints) already watches the settings file; let an app register paths (`fx.watch(key, path)` / `fx.unwatch(key)`) and receive `Event::FileChanged { key }`. Notes watches the open notes' files and shows the conflict the moment it is seen, offering *Keep mine (save a copy)* / *Take theirs* — never last-writer-wins |
| **Recovery after a forced kill** | The journal covers a kill between the content and the metadata commit | A kill between keystroke and autosave (up to 750 ms, or longer if a save is failing) loses typing | App only: the framework's **drafts** (`fx.save_draft`, debounced, read back from `Boot.drafts`), one draft per open note, discarded when its save lands. Test: type, kill (drop the app without shutdown), start again — the text is offered back |
| **Close cancelled** | `Event::CloseRequested(window)` exists; returning a command keeps the window open | Notes closes regardless | App only: a dirty note on close shows *Save / Discard / Cancel* in the kit's dialog; Cancel keeps the window and the text; Save closes once the save lands; Discard closes and discards the draft |
| **Two document contexts** | `WindowId` is in the API (`render(ui, window, …)`, `CloseRequested(window)`, `fx.close(window)`), and B1 says nothing may assume one window | The core holds one `Ui` and wires one window | **Framework: secondary windows.** `fx.open_window() -> WindowId` makes another `A::Ui`, themes it, connects its chrome, calls `A::wire_window(cx, id)`, and renders it; `render` is called per window; `Cx::ui_for(id)`; closing a secondary window ends only that window; closing the primary ends the app only when no secondary is dirty (each window's close goes through `CloseRequested`). Notes: *Open in new window*, each window with its own current note, undo stack and draft; both windows editing the same note share one model note, so they can never diverge |
| **Interrupted migration** | The framework's settings store migrates by schema, imports a pre-framework file once with a backup kept, and enters protected recovery for a newer schema; step 4's tests and Weather's `a_file_from_before_the_framework_is_imported_once_and_kept` exercise it, including a rewrite owed and not yet made | Notes has nothing to migrate: no settings (its folder, focus mode and preview live in the window and reset each start), and a vault that has been `v2` since its first commit | ★ **No invented migration.** Inventing settings or a vault v3 for the gate would be speculative work that ships a migration nobody needs. The vault gains a format marker (`.vault-version`, written once, read at load, a newer one refused without touching the vault) so the first real v3 has something to read and resume from. The framework's app-data migration helper is built with that v3, not before. Persisting the view state (a real improvement) belongs to Notes' redesign PR, where a behaviour change is allowed, and its schema then starts at 1 |

★ The two framework additions — **secondary windows** and **watched paths** — are the only framework changes. Each is its own PR on `frameworks/yantrik-app` with tests in `examples/hello-app` (a second window; a watched file edited outside), reviewed before Notes uses it.

## Order of work

1. **8a — watched paths** (framework): `fx.watch`/`fx.unwatch`, `Seen::File(key)` from the watch thread, `Event::FileChanged { key }`; self-write echo suppression by fingerprint as the settings watch does. Hello test.
2. **8b — secondary windows** (framework): the core's window table, `fx.open_window`, `App::wire_window` (default: `wire`), per-window render, per-window close, `TestApp` able to open and drive a second window headless. Hello test.
3. **8c — Notes on the framework, behaviour unchanged** (parity PR, like Weather's): the store moves as is; the worker becomes jobs on one key (saves stay ordered); `describe`, the actions and their grades identical; the store's tests and the `tests.rs` move with it.
4. **8d — the gate:** the six items above on Notes, each with its test through `TestApp` and a real vault directory, and 520 evidence for the two a person sees (the conflict prompt, the close prompt).

Nothing else moves onto the framework until 8d passes; the four dashboard ports (#900, #901, #903, #904) use none of 8a/8b and do not wait.

## What would change the plan

- If 8b shows Slint cannot run two windows of one component well under the shell's compositor (focus, chrome, theme), Notes' second context becomes a second *tab* in one window and the framework keeps one window per app — said in the gate PR, not hidden.
- If watching a 2,000-note vault by polling costs more than a few milliseconds per tick, only the open notes are watched (the plan above already watches only those).
