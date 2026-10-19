# Built-in Apps

Yantrik OS ships a suite of productivity and system apps, launched from the dock or the
Apps screen.

Every app has AI integration — you can ask the companion to help with tasks directly inside each app, or use the dedicated AI panels built into the office apps.

> **Nothing is shelved in this build.** Music and ySheets were shelved on 20 September 2026,
> because neither had anything under its screen, and both came back in October 2026 — ySheets
> in #727 and Music in #728. The account is
> [`design/shelved-2026-09-20.md`](../design/shelved-2026-09-20.md), and the list the shell
> actually reads is `SHELVED` in `crates/yantrik-ui/src/wire/dock.rs`, empty today.

## Productivity Apps

### ySheets (Spreadsheet)

A sheet of numbers and formulas, opened from and saved to CSV files. Shelved on 20 September
2026 because there was no cell model behind the grid; back since October 2026 (#727).

**Features:**
- A cell model that is always `rows × cols`; type into a cell or the formula bar.
- Formulas with references and dependency-ordered recalculation, loop detection, and
  `SUM`/`AVERAGE`/`MIN`/`MAX`/`COUNT` among the functions.
- CSV open and save through the desktop's file picker, or a file passed to it (`%f`).
- Undo and redo.
- A control surface, `spreadsheet` (aliases `ysheets`, `sheets`), so a mind can read and edit a
  sheet with `yos act spreadsheet …`; see [`app-control.md`](app-control.md).

ySheets saves what was typed, formulas included, so a cell of an opened CSV that starts with `=`
is saved back unchanged — and Excel or LibreOffice will read it as a formula.

Everything else the old screen drew without backing (charts, comments, formatting, the AI panel)
was removed rather than kept as a drawing.

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

### Music

Plays the music in your folders through mpv. Shelved on 20 September 2026 because nothing
played; back since October 2026 (#728).

**Features:**
- A library from the folders you list under **Folders** (`~/Music` on a first run): every
  mp3, flac, ogg, opus, wav and m4a file, titled by its file name, grouped by its folder. The
  scan runs in the background; linked folders inside are not followed.
- Search the library; pick a track and the list becomes the queue. The next track plays when
  one ends.
- Play/pause, stop, next/previous, seek, volume and mute, drawn from what mpv reports.
- Opens a sound file passed to it (`yantrik-music-player song.flac`, or `%f` from a launcher);
  a second launch hands the file to the open window.
- If mpv is missing or stops, the window says so; the next play starts it again.
- Driven by a mind as `music`: `yos act music play index=1`, `pause`, `next`, `previous`,
  `seek seconds=30`, `set_volume percent=40`, `play_file path=~/Music/song.flac`, `rescan`.

Not here, because nothing would back them yet: playlists, artist/album/genre views, shuffle,
repeat, speed and an equaliser. Tags are not read.

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
