# Built-in Apps

Yantrik OS ships with a full suite of productivity and system apps. All apps are accessible from the app dock on the desktop.

Every app has AI integration — you can ask the companion to help with tasks directly inside each app, or use the dedicated AI panels built into the office apps.

## Productivity Apps

### ySheets (Spreadsheet)

A full spreadsheet with formula engine, formatting, and AI-powered data generation.

**Features:**
- Formula engine with standard spreadsheet functions (`=SUM()`, `=AVERAGE()`, `=IF()`, etc.)
- Multi-sheet tabs
- Cell formatting: bold, italic, text color, background color, number formats
- Column resizing, row/column headers
- Find & replace
- AI panel:
  - **Analyze** — AI analyzes selected data and provides insights
  - **Generate** — Create entire datasets from a description (e.g., "quarterly sales data for a SaaS company")
  - **Formula help** — Explain or suggest formulas
- Real-time cell preview — typing in the formula bar immediately reflects in the active cell
- Auto-persist — clicking another cell automatically commits the current value (no Enter required, like Excel)
- Undo/redo support

**AI Generate example:**
Type "Employee directory with departments and salaries" in the AI panel and click Generate. The companion creates a realistic CSV dataset and populates the sheet.

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

### Music Player

Audio playback with playlist management.

**Features:**
- Play local audio files
- Playlist creation and management
- Playback controls (play, pause, skip, volume)
- Now playing display

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
- Shell access (`/bin/ash` on Alpine)
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
- Install and remove packages (uses Alpine's `apk`)

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

See [CONTRIBUTING.md](CONTRIBUTING.md) for details on adding new apps.
