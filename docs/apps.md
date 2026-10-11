# Built-in Apps

Yantrik OS ships a suite of productivity and system apps, launched from the dock or the
Apps screen.

Every app has AI integration — you can ask the companion to help with tasks directly inside each app, or use the dedicated AI panels built into the office apps.

> **Two apps are shelved and are not in this build: ySheets and Music.** They are described
> below, marked, rather than quietly deleted, so that a page promising them cannot outlive the
> decision. The account is [`design/shelved-2026-09-20.md`](../design/shelved-2026-09-20.md)
> and the list the shell actually reads is `SHELVED` in
> `crates/yantrik-ui/src/wire/dock.rs`.

## Productivity Apps

### ySheets (Spreadsheet) — SHELVED, not in this build

**Shelved on 20 September 2026.** Asking the shell to open it gets a refusal naming the
reason, not an error; it is not in the launcher, not in the Lens, not offered to a mind, and
not shipped in the release bundle or the ISO.

**Why:** there is no cell model behind the grid, so nothing can be typed into it, by mouse or
by mind. The screen drew a perfect 50×26 grid over 1249 lines of Slint and 156 lines of Rust
in which `cell-grid` was never written, `row-count` and `col-count` were always 0, and every
handler that touches a cell took an early return. There is no formula engine — `is_formula`
tested whether text *starts with* `=`, and nothing evaluated one. Save and Load were log
lines.

**What brings it back:** a cell model, CSV load and save, and arithmetic with references plus
`SUM`/`AVG`/`MIN`/`MAX`/`COUNT`.

The crate stays in the tree and stays a workspace member, so it keeps compiling. What changed
is that a person cannot reach it and a mind is not told it exists. Nothing in the desktop
routes `.csv`, `.xlsx` or `.ods` at it — `.csv` opens in the text editor.

### yPresent (Presentations)

A presentation editor with AI deck generation, templates, and slideshow mode.

**Features:**
- Slide editor with title, body text, and speaker notes
- Slide thumbnails sidebar with drag reordering
- Template gallery (Title Slide, Content, Two Column, Image, Quote, Section Header, Blank)
- AI panel:
  - **Generate deck** — Create an entire presentation from a topic description
  - **Improve slide** — Rewrite current slide content
  - **Add speaker notes** — Generate notes for the current slide
- Presentation mode (fullscreen slideshow with keyboard navigation)
- Slide transitions
- Find & replace across slides
- Export support

**AI Generate example:**
Type "Company quarterly review for Q1 2026" and click Generate. The companion creates a multi-slide deck with title slide, agenda, highlights, metrics, challenges, and next steps.

### yDocs (Document Editor)

A rich text document editor with AI writing assistance.

**Features:**
- Rich text editing (bold, italic, headings)
- Word count and character count
- AI writing assistance:
  - Summarize text
  - Expand on ideas
  - Fix grammar and style
  - Generate content from prompts
- Auto-save

### Notes

A lightweight note-taking app for quick capture.

**Features:**
- Create, edit, and delete notes
- Search across all notes
- Timestamps and sorting

## Communication Apps

### Email

An IMAP email client with AI-powered triage.

**Features:**
- IMAP email fetching (configurable server)
- Email list with sender, subject, date, and preview
- Read/compose/reply
- AI-powered triage — the companion identifies important emails and surfaces them as notifications
- Smart notification suppression — learns which email types you don't care about

**Setup:**
Configure your IMAP server in Settings → Email, or ask the companion: *"Set up my email"*

### Calendar

Event management with schedule awareness.

**Features:**
- Monthly/weekly/daily calendar views
- Create, edit, and delete events
- Time-based reminders
- The companion is schedule-aware and can warn about conflicts or upcoming deadlines

## Media Apps

### Music — SHELVED, not in this build

**Shelved on 20 September 2026**, the same way and for the same kind of reason as ySheets:
refused by name with the reason, absent from the launcher, the Lens and the release bundle.

**Why:** nothing plays audio yet — there is no playback engine, no scanner and no library
behind the screen. 2168 lines of Slint were driven by 312 of Rust, of which 23 of 34 handlers
did nothing, including every one that would make a sound. Double-clicking a song logged
`"Play track index N (stub)"`. The only way a track ever appeared on that screen was
`YANTRIK_MUSIC_DEMO=1`, which fills the library with twelve invented tracks and is honestly
labelled a design fixture.

**What brings it back:** mpv driven over its JSON IPC socket, a folder scan filling a small
library store, and play/pause/next/queue and `open <file>` working.

**Audio does still play on this machine** — that is a different claim, and worth separating.
The shell's own media screen (below) drives a real `mpv` over a real IPC socket, and every
audio file type the desktop classifies routes to it. It never routed to the Music app.

### Image Viewer

View images with basic navigation.

**Features:**
- Open and display image files
- Zoom and pan
- Navigate between images in a directory

### Media Player

Video and media playback.

## System Apps

### Files

A file browser with AI-assisted organization.

**Features:**
- Browse directories
- File operations (copy, move, delete, rename)
- File previews
- Ask the companion about files: *"What's taking the most disk space in my Downloads?"*

### Terminal

A built-in terminal emulator.

**Features:**
- Shell access (`$SHELL`, falling back to `/bin/bash` — this is Debian 13)
- Full terminal emulation
- The companion can help with terminal commands — ask *"How do I find large files?"*

### System Monitor

Real-time system metrics and process management.

**Features:**
- CPU usage (per-core and aggregate)
- RAM usage and swap
- Disk space usage
- Network I/O
- Process list with CPU/memory per process
- Kill processes

### Network Manager

WiFi and ethernet configuration.

**Features:**
- View network interfaces and status
- Connect to WiFi networks
- View IP addresses and connection details

### Package Manager

System package management.

**Features:**
- Browse installed packages
- Search for available packages
- Install and remove packages (Debian: `apt-get`, through `sudo`)

### Weather

Local weather and forecasts.

**Features:**
- Current temperature and conditions
- Multi-day forecast
- Configurable location

## System Screens

### Settings

Central configuration for the entire system.

**Sections:**
- General — user name, companion name, language
- AI — LLM backend, model selection, temperature
- Privacy — tool permissions, data retention
- Appearance — theme selection, colors
- Notifications — urgency thresholds, quiet hours
- Email — IMAP server configuration
- Updates — release channel, auto-update

### About

System information and version display.

- Shows all component versions (yantrik-ml, yantrikdb, yantrik-companion, yantrik-os, yantrik-ui)
- Git commit hash and build date
- Check for updates button

### Lock Screen

Lock the system with a PIN or pattern.

### Onboarding

First-time setup wizard — introduces the companion, configures preferences, and personalizes the system.

## Adding Apps via AI

The companion can help you launch external apps too:

- *"Open Firefox"* — launches Firefox if installed
- *"Open a terminal"* — opens foot terminal
- *"Take a screenshot"* — captures the screen via grim

## App Wiring Pattern (for developers)

Each app consists of:
1. **UI definition** — `.slint` file in `crates/yantrik-ui/ui/`
2. **Backend logic** — `.rs` file in `crates/yantrik-ui/src/wire/`
3. **Registration** — entry in `apps.rs`, `app.slint`, `wire/mod.rs`, and the dock

See [CONTRIBUTING.md](../CONTRIBUTING.md) for details on adding new apps.
