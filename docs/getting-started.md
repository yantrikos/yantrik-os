# Getting started

Yantrik OS is a **live ISO**. You download an image, boot it, and the desktop runs from the
image without touching any disk. When you decide you want it on a disk, the installer is
inside the running desktop.

There is nothing to add to a Linux system you already have. An older version of this page
told people to install Alpine Linux first and pipe an installer into a shell; that has not
been how this works for months, and the domain that installer fetched from no longer
resolves.

Before anything else, what you are getting into: this is early software published on a
**nightly** channel and nothing else. Things break. The audits of what does not work — app by
app, screen by screen — are in [`design/`](../design/) in this repository. Read
[`design/shelved-2026-09-20.md`](../design/shelved-2026-09-20.md) for two apps (Music and
ySheets) that were taken out of the build for being drawings of apps, and for how they came
back.

---

## 1. Download the image

One link, always the newest nightly: **<https://iso.yantrikos.com/latest.iso>**. It leads to
`https://iso.yantrikos.com/nightly/yantrik-os-latest.iso`, which the release host re-points at
each build as it is published, and its checksum file names it, so three lines fetch and check it:

```bash
curl -fL -O https://iso.yantrikos.com/nightly/yantrik-os-latest.iso
curl -fL -O https://iso.yantrikos.com/nightly/yantrik-os-latest.iso.sha256
sha256sum -c yantrik-os-latest.iso.sha256
```

Which build that is today, with its sha256, size and version, is in
`https://iso.yantrikos.com/nightly/latest.json`. The host keeps only the newest few builds, so
never link a build's own filename: it is gone a week later.

`install.sh` in this repository does the same thing for you and is also served from
`https://get.yantrikos.com/install.sh`:

```bash
sh install.sh               # prints the url, size and sha256 of the current image, and stops
sh install.sh --download    # also fetches it here and checks its sha256
```

Piped straight into a shell with no flag it explains and does nothing else: it writes no
file, installs nothing and wants no privilege. That is deliberate, so that nothing runs from a
pipe before you have read it.

The image is about **1.85 GiB** as built (1,984,960,512 bytes for build
`v0.1.0-2690-g92eec0bd`, 16 Nov 2026). The size changes from build to build; `bytes` in
`latest.json` is the exact size of the build you are about to fetch. If you want to keep the
exact build you tested, keep the file: the `latest` name will mean the next build tomorrow.

**Verify the checksum before you boot it.** The point is not that someone is attacking you;
it is that a download of nearly 2 GiB that ends at 98% gives you an image that boots halfway and
fails in a way you will spend an afternoon on.

Only the **nightly** channel has builds. `beta` and `stable` exist as names in the updater
and in the publishing pipeline, and nothing has ever been published to either —
`https://iso.yantrikos.com/stable/latest.json` is a 404. If you see a stable channel
mentioned anywhere, it is aspirational.

### Where the image comes from

It is built by [`.github/workflows/iso.yml`](../.github/workflows/iso.yml) on a GitHub
runner: the whole workspace is compiled in release, packaged by
[`build-release.sh`](../deploy/yantrik-os/build-release.sh), assembled into a Debian 13
(trixie) live image by
[`build-debian-iso.sh`](../deploy/yantrik-os/build-debian-iso.sh), and then **booted** under
QEMU by [`boottest.py`](../deploy/yantrik-os/boottest.py), which waits for the machine to
reach its login on the serial console and answer for itself. An image that does not boot is
not published. Nobody's laptop is in that path.

That test is a BIOS boot under QEMU with a virtio display. The UEFI path is built and CI does
not boot it, but it has been run by hand: the image installed and booted under UEFI firmware
(OVMF) in a virtual machine. That is all that has been tried. Real hardware of any kind is not
tested at all.

---

## 2. Try it in a VM first

This is the recommended way to meet it, and it is what the project develops against.

**Settings that are known to work** — the project's own test machine, as it reports itself:

| | |
|---|---|
| Platform | QEMU/KVM, `Standard PC (Q35 + ICH9)` |
| CPU | 4 vCPU (no passthrough, no host CPU features assumed) |
| RAM | 8 GB |
| Disk | 32 GB |
| GPU | none — virtio-gpu, software rendering through Mesa's llvmpipe |
| Firmware | BIOS (SeaBIOS) in CI and on the project's test VM. UEFI: installed and booted under UEFI firmware (OVMF) in a virtual machine |

CI's automated boot test is smaller and still works: QEMU with **4 GB of RAM, 4 CPUs** and
`-device virtio-vga`, KVM when the runner has `/dev/kvm` and plain emulation when it does
not.

A minimal QEMU invocation to try it yourself:

```bash
qemu-system-x86_64 -enable-kvm -m 4096 -smp 4 \
  -cdrom yantrik-os-<version>.iso -boot d \
  -device virtio-vga -display gtk
```

VirtualBox ought to work — this is an ordinary Debian live ISO — but nobody here has checked
it against a current build, so it is not on the list above. If you try it, say how it went.
The one setting this repository gives it (configured, not measured):
[`build-debian-iso.sh`](../deploy/yantrik-os/build-debian-iso.sh) says to import the ISO as
boot media with **4 GB of RAM and EFI enabled**. (`deploy/yantrik-os/legacy/build-vbox-image.sh` (not shipped) builds an Alpine-era VM
and is history; no current path uses it.)

On Proxmox, give the VM a **VirtIO display** (`vga: virtio`) and, for UEFI, an OVMF firmware
with an EFI disk. The display is the setting that is not cosmetic: with Proxmox's default the
machine boots, reports a healthy session, and shows a black screen forever, because there is no
`/dev/dri` for labwc to find. (The repository's
[`proxmox-deploy.sh`](../deploy/yantrik-os/proxmox-deploy.sh) is a different path: it turns a
Debian cloud image into Yantrik OS on first boot, with 8 GB / 4 cores / a 24 GB disk.)

## 3. …or write it to a USB stick

```bash
# Linux and macOS. /dev/sdX is the DEVICE, not a partition. Getting it wrong overwrites
# the wrong disk. `lsblk` on Linux, `diskutil list` on macOS (where it is /dev/rdiskN and
# the stick must be unmounted first with `diskutil unmountDisk`).
sudo dd if=yantrik-os-<version>.iso of=/dev/sdX bs=4M status=progress conv=fsync
```

On Windows use **Rufus** (<https://rufus.ie>) or **balenaEtcher**
(<https://etcher.balena.io>). On macOS without `dd`, balenaEtcher does the same job. All of
them take the `.iso` exactly as downloaded — do not unpack it.

---

## 4. Boot it

A boot menu shows for five seconds, then the default entry, **Install Yantrik OS**, boots. It has
four entries:

| Entry | What it does |
|---|---|
| **Install Yantrik OS** | Boots live *and* puts first-run setup into installer mode, so it offers to write to a disk at the end |
| **Try Yantrik OS without installing** | Boots live. Nothing offers to touch a disk |
| **Install Yantrik OS with safe graphics** | The same as the first, with `nomodeset` — use this if the screen stays black |
| **Try Yantrik OS, showing every boot message** | No `quiet`, no `plymouth` splash and `nomodeset`, so the kernel and systemd say what they are doing. Every entry also writes to the serial line. This is the entry to boot from when you are filing a bug |

Either way the machine boots to a live desktop first. Nothing is written to any disk until
you tell the installer to erase one.

Two things about the live session that are true and worth knowing before you put it on a
network:

- The live user is `yantrik` with the password `yantrik` and passwordless sudo. On a live
  image, anyone with physical access is root anyway. The disk installer asks you for a real
  password.
- SSH is installed and **disabled**. It is not turned on by any boot entry.

### What you see first

Setup happens in two parts, one on each side of the install. This is what a visitor following
only the README saw on 16 Nov, in a virtual machine under UEFI firmware.

**The installer's pages come first** (the "Install" entries). One `grub.cfg` serves BIOS and
EFI, so the five-second boot menu is drawn on both, then the Welcome page comes up. The visitor
run did not see the menu, most likely because the countdown was missed.

1. **Welcome**: the keyboard layout, and **Try it first** or **Install**.
2. **About you**: your name, a username and a password. The computer's name is worked out from
   the username and sits behind a small **More** link if you want to change it.
3. **Where to install**: the disk, and **encryption, which is ticked by default** ("Encrypt with
   my password"). With it ticked, the installed machine asks for the password at every start, on a
   screen that shows your name; without it nobody can read the disk, you included: a forgotten
   password means an unreadable disk. To turn it off, **untick it on this page**; the page then
   says anyone who takes the disk out can read everything. Each disk is listed in general terms
   (for example "3 partitions (vfat, ext4, crypto_LUKS)"), and in erase mode every disk carries
   "Everything on the disk you choose will be erased."
4. **Ready to install**: every answer and the disk together, before anything is written, with a
   **Thinking** row (Choose or Change) where you can pick a model provider now. "Erase disk and
   install" starts the install.

Then it installs (about five minutes on the test VM; the page says "about five minutes") and
restarts.

**Take the installer disc out before the restart.** The last page says "The disc comes out as the
computer restarts", but in a virtual machine it does not
([#1165](https://github.com/yantrikos/yantrik-os/issues/1165)). Detach the ISO, and check the
boot order. A machine that still has the ISO attached starts the installer's Welcome page again,
which looks like the install looped. The install itself is fine. On real hardware with a USB
stick, the page says to take it out once the screen goes dark.

**First run comes after the restart**, on the installed machine's first boot, once you have
entered the disk password (if you left encryption on). It has four pages, and it asks nothing
about your interests or where you are. If you choose a provider on it there are five (a model on
this computer skips the sharing page, so it stays at four). If you chose
one in the installer's Thinking row there are three:

1. **Welcome**: text size and read-aloud options, then Get started. On a boot that did not come
   through the installer it also asks "What should your minds call you?".
2. **Who should do the thinking?**: where the AI runs. See section 5 for what this page offers
   today. "Not now" skips it.
3. **Getting your minds ready**: a check of each mind. If you chose no model it says "Needs
   attention: it is not set up to think yet".
4. **Your minds**: one row per mind with what was seen, and "Add another mind".

<!-- SWAP-ON-#1274: with "Start free", this page is skipped on that path; consent is on the card. -->
If you choose a provider, a page about what your minds share with it (a sentence naming the
provider, ticked by default) comes after the provider page. The page's own sentence, "what they
remember about you, and where you are", is narrower than what is sent
([#1275](https://github.com/yantrikos/yantrik-os/issues/1275); see section 5). The 16 Nov run
chose none, so it did not see that page; it is described here from the code. The desktop works
without a model.

On the second page, the 16 Nov run found the page taller than the screen: wheel events sent to the
virtual machine did not scroll it, and Tab did. That was not tried with a real mouse.

Behind it is the shell: a status bar, a dock, and the Lens, which is where you type to the mind.

The keys are the ones Windows, GNOME and Ubuntu agree on, so they are probably already in your
fingers. The full set is in [`config/labwc/rc.xml`](../config/labwc/rc.xml); these are the ones
worth knowing on the first day:

| | |
|---|---|
| `Ctrl`+`Alt`+`T` | Terminal |
| `Super`+`K` | The Lens, from inside any window |
| `Super`+`E` | Files |
| `Super`+`I` | Settings |
| `Super`+`D` | Desktop |
| `Super`+`L` | Lock |
| `Alt`+`Tab` / `Alt`+`F4` | Cycle windows / close |
| `Super`+`←`/`→`/`↑`/`↓` | Snap, maximize, minimize |
| `Print` | The capture bar: a screenshot or a recording, of a region or the whole screen |
| `Shift`+`Print` / `Super`+`Shift`+`S` | Screenshot of a region you drag, into `~/Pictures/Screenshots` and the clipboard |
| `Ctrl`+`Print` | Screenshot of the whole screen, to the clipboard only |
| `Super`+`Shift`+`R` | Stops a screen recording that is running (saved in `~/Videos/Recordings`); with none running it opens the capture bar on Record |

---

## 5. Attach a mind

**The image ships no language model and no mind.** That is deliberate and it is the thing
people are most surprised by. The desktop is the desktop; what answers in it is something you
point it at.

The default configuration (`/opt/yantrik/config.yaml`) has an empty `api_base_url`: it points at
nothing, because an address baked into a public image is an address every copy of that image
would talk to. Only a build made with `--with-llm` writes a loopback endpoint (port 8341) there.

### What the first-run provider page offers today

<!-- SWAP-ON-#1274: this paragraph and the "Not now" paragraph below change: #1274 adds a keyless
"Start free" card, and a "Start free" chip beside "Add key". -->
Every card on the "Who should do the thinking?" page needs an account: Groq ("free, fast open
models, needs a free Groq account", marked Recommended), Claude, OpenAI, Google Gemini, DeepSeek,
OpenRouter and NVIDIA NIM. "Show every provider" lists more, and "Advanced" takes an endpoint of
your own. "Look on your network for a model already running" finds one only if you have one. A
visitor with no account and no local model has no choice on that page except "Not now".

If you choose "Not now", the next page says "Needs attention: it is not set up to think yet", and
the desktop's ask bar says it does not know where the companion sends your words and offers "Add
key". The desktop still works.

<!-- SWAP-ON-#1167: this paragraph is true today. A change that puts the free models in first run
is in progress under #1167. When it lands, replace this one paragraph (and the marked blockquote in
README.md) and nothing else. -->
**Free models that need no account.** They exist, and today you reach them in Settings, not in
first run. After "Not now": (1) Settings, AI & Intelligence, AI accounts card: turn on "Let your
Minds that use Yantrik models share what they know about you with OVHcloud AI Endpoints" (the
16 Nov run used OVHcloud). What you type then goes to that provider, with your conversation; what your minds remember about you, including notification text, file names and apps opened that the shell wrote by itself; your name and the local time; on every turn the computer's state (network name, running apps, battery, CPU, memory, disk, idle time); and the results of every tool the model uses (files, clipboard, window text, browser pages, command output, described screenshots). This applies to every mind on Yantrik models
([#1275](https://github.com/yantrikos/yantrik-os/issues/1275)). (2) Settings, Minds: press **Use
Yantrik models** on the Yantrik Companion row, then Apply. (3) Pick a model in the ask bar; after
Apply it still says "Pick a model" until you do. With this a fresh machine answered a question
with no key. In that run some of the text on the Settings page was cut off at the right edge,
including most of the **Use Yantrik models** button; the visible part can be clicked.

<!-- SWAP-ON-#1274: this sentence says the change is "not in the image yet". -->
A change that puts these free models in first run is in progress
([#1167](https://github.com/yantrikos/yantrik-os/issues/1167)). It is not in the image yet.

### The three shapes of answer

Once you are past that page, you have three shapes of answer:

1. **A model you run yourself.** Ollama or llama.cpp on the same machine or elsewhere on
   your LAN. Nothing leaves your network.
2. **A provider you choose.** Any OpenAI-compatible endpoint, or the Claude CLI. What you
   type goes to it, and so does what the companion adds to each turn: memories, the computer's
   state and tool results ([#1275](https://github.com/yantrikos/yantrik-os/issues/1275)). That is
   more than most clients send.
3. **A separate agent that attaches to the desktop.** Yantrik Mind, Hermes, Pi, DeepSeek and
   OpenClaw all do this; the last four ship as source in [`harnesses/`](../harnesses/). The
   OS does not hold the harness's endpoint, model name or key — the harness dials in and keeps its
   own configuration (the one exception is "use a saved provider" in Settings, which copies a key
   from the credential vault into that harness's settings, and can be reverted). See
   **[harness.md](harness.md)** for the protocol and how to write one.

<!-- SWAP-ON-#1274: "set up after a model is chosen" changes: Start free does not start Hermes. -->
Hermes, the default Mind, is set up after a model is chosen. It downloads about 3 GB (from GitHub
and Python's package index), so it needs the internet. Until it is ready the built-in Companion
answers ([#1166](https://github.com/yantrikos/yantrik-os/issues/1166)). The first-run pages carry
this note even where no model was chosen; the 16 Nov run chose none, so did not test the download.

Whichever you pick, the desktop says which mind is answering: the Lens's conversation names it.
The Minds panel, opened from the mind chip, shows "Answering: <mind>" and what it runs on, the
provider and model, for instance "Ollama Cloud · deepseek-v4.1-flash". The Mind panel on the right
edge (hidden until you pin it, for instance with `yos act shell set_mind_panel shape=pinned`)
says "answering · <account> · <model>", as do, since PR #1162, the Lens line and the chat bubble.
This tells you which provider a turn goes to. It does not list everything that goes with the
turn (see above).

### How much it may do without asking

The **mode chip** is in the right-hand group of the top bar, to the left of the network and sound marks. It
reads "Mind · " and a phrase: **Suggests only** (`plan`), **Asks first** (`ask`), **Acts within
limits** (`auto`), or a bypass's own label with its countdown ("Full bypass · …"); a mind in
private mode shows **Private**. Click it for the menu, which includes "See what it did without
asking". The modes:

| mode | the mind may… |
|---|---|
| `plan` | read only — every change is refused and it has to tell you what it *would* do |
| `ask` | routine things run; sensitive ones put a card in front of you (the default) |
| `auto` | sensitive things run; you are still asked about destructive ones, and once per session before it runs commands |
| `bypass` | nothing is asked except before anything an app marks as impossible to undo, such as a purchase or a calendar delete — that still puts a card up. Commands run unasked and are audited. Time-boxed, and never written to disk, so no machine boots into it |
| `bypass_all` | "Full bypass": nothing is asked at all, those included, with two exceptions: once a mind connected to this desktop has read your private data, its command, hand-off to another mind, new web address or send of text off this machine still puts a card up; and the three actions that always ask (location lookups, background research, a problem report) still put one up, and so do the built-in assistant's four questions (it cannot delete or overwrite a file, send a chat message, plan work, or change a mail, an event or a memory without a card you pressed, in every mode). The built-in assistant's own mail is not covered: in `bypass_all` it sends without one. Time-boxed and never written to disk, like `bypass`; chosen on the same confirmation |

A command — the Terminal's `run`, an agent's own terminal — can do anything you can, so it asks
before it runs in `ask`, `auto` and `bypass`. Press **Allow for this session** on that card and
the mind's commands after it run without asking until the shell restarts.

What a Mind or harness ran without a card is written down in
`~/.local/share/yantrik/mind-audit.jsonl` and readable from the mode menu ("See what it did
without asking"). (#1262: that log can stop writing.) The built-in companion's own tool calls
are not in that file. They are recorded as `audit/tools` memories in YantrikDB, with a seven-day half-life
(they fade from recall), at most 200 characters of each result kept, and not written in incognito. At the shipped setting the companion runs Safe, Standard and
Sensitive tools with no card, shell commands and screen reads included. To restrict it, edit
`tools.max_permission` in `/opt/yantrik/config.yaml`, then restart the session; the Settings
control "Tool permission" sets the ceiling for Minds and harnesses and reaches the companion only
through `app_action`. See
[SECURITY.md](../SECURITY.md).

### Stopping a mind

The Lens has a **Stop** control (PR #1072), and typing **`/stop`** does the same: it is never
queued as a message, it goes straight to the running turn and ends it, and closes any question the
mind was waiting on. A Hermes run that is already working may carry on after the desktop has
stopped waiting for it; Hermes honouring Stop is PR #1161 and is not merged.

---

## 6. Install it to a disk

**This erases the disk you point it at.** Use a machine, or a VM, you can afford to wipe. The
disk installer has not been through the boot test that the image itself has — treat it as the
least-proven part of this.

The target disk should be at least **6 GB**. That figure is stated here and in the README's
hardware table, and a test holds the table to the first-boot hardware scan. Nothing refuses a
whole disk under 6 GB, though. Installing beside another system enforces 20 GB, and nothing
checks the firmware. The scan measures the whole size of the disk the picker has selected; it
used to report the free space of the live session, which on a live USB describes the stick you
booted from and not the disk you are about to install to.

Two ways in. They install the same system, but the text installer does not encrypt the disk:

- **The graphical installer.** Boot the "Install Yantrik OS" entry: the four pages described
  under "What you see first" (welcome, you, disk and encryption, review).
- **The text installer.** In a terminal in the live session:

  ```bash
  sudo /opt/yantrik/bin/yantrik-install
  ```

  The full path is not an affectation. `yantrik-session` puts `/opt/yantrik/bin` on your
  `PATH`, so plain `yantrik-install`, `yantrik-update` and `yos` all work — but `sudo` on
  Debian replaces `PATH` with its own `secure_path`, which does not include that directory, so
  `sudo yantrik-install` is "command not found". Anything you run through `sudo` here wants
  the full path.

  It is [`deploy/yantrik-os/yantrik-install.sh`](../deploy/yantrik-os/yantrik-install.sh) in
  this repository, installed into the image as `/opt/yantrik/bin/yantrik-install`.

It asks for your name, a username, a password, a hostname and a timezone (asked, never guessed
from the network; the default is the live system's zone, normally UTC), then shows you every answer and the disk together and
waits for you to type `yes`. Then it partitions (GPT, EFI or BIOS depending on how you
booted), copies the live filesystem to the disk, removes the live-boot packages, creates your
account, installs GRUB and reboots.

After that the machine boots to the desktop with no display manager, and the shell starts
locked: unlock it with your password. (If you left encryption on in the graphical installer, that password is also asked
at the very start, to open the disk. The text installer never encrypts, so it asks for nothing there.) The first run described above happens once, on that first
boot. The screen locks by itself after about five minutes idle.

---

## 7. Update it

An installed machine updates itself with **`yantrik-update`**:

```bash
yantrik-update check        # what is installed, versus what the channel has
yantrik-update apply        # download, verify, install, restart the session
yantrik-update rollback     # restore the previous build
yantrik-update status       # channel, host, current build, backups on disk
yantrik-update set-channel nightly|beta|stable
```

`apply` takes `--force` (reinstall the same build), `--allow-downgrade` (install the channel's
build even though it is older than the one here), `--reboot` (instead of restarting the
session) and `--no-restart`. `check` and `status` take `--porcelain`, which is what the
desktop's About screen reads.

What it does for you: the update information is a signed file (ed25519; the public keys ship
with the build and are scoped per channel), checked before any field of it is read — an
unsigned, expired or older-serial file is refused and nothing is installed (`check` says
"not signed by Yantrik" and exits 14). The bundle is then verified against the signed
manifest's sha256 before a single file is replaced, the current binaries and shared files are backed up first, and if the new
shell does not answer its control socket after the restart the update rolls itself back. The
restart goes through the session unit rather than respawning the shell by hand, so the
desktop comes back exactly as it does on boot.

Again: `nightly` is the only channel with builds on it. `set-channel stable` will succeed and
then `check` will tell you the channel is not published, which is the honest answer rather
than an error.

A machine can be *ahead* of its channel: a developer deploy from `main`, or a beta build on a
machine moved back to nightly. `check` compares where the two builds sit rather than whether
their commits differ, so it says the machine is ahead and offers nothing to install — an older
build is not an update, and the About screen shows no Install button for one. Moving to it
anyway is a downgrade, and `apply` only does that when `--allow-downgrade` says so out loud.

`yantrik-update` is not an app updater. Software you install with `apt` is yours to update
with `apt`.

---

## 8. Driving it from a terminal, or from an agent

Every app and the shell itself publish a control surface — what they are showing and what can
be done to them:

```bash
yos describe shell                          # where you are, what is open, what is wrong
yos act shell open_app name=notes           # launch or focus an app
yos describe notes                          # any running app answers for itself
yos ls                                      # which surfaces are live right now
```

`yos-mcp` exposes the same surface over MCP, so an agent that speaks MCP can drive the
desktop through the same path a person uses. See [app-control.md](app-control.md).

---

## Troubleshooting

### Black screen after boot

Boot the **Install Yantrik OS with safe graphics** entry, which adds `nomodeset`. If you have installed to disk already,
the renderer settings are in `/etc/yantrik/session.env` (root's; edit it with `sudo`, and reboot) —
`WLR_RENDERER=pixman`, `LIBGL_ALWAYS_SOFTWARE=1` and `SLINT_BACKEND=winit` are what a machine
with no usable GPU wants.

### Nothing answers when I type

Check the endpoint you gave it during setup is actually reachable from the machine:

```bash
curl http://<your-host>:11434/api/tags      # Ollama
tail -f /opt/yantrik/logs/yantrik-os.log
```

If you attached a harness rather than configuring a provider, the harness has to be running
and polling — the desktop never connects out to it.

### Answers take forever

Small models on a CPU with no GPU produce a token or two a second, which is not a
conversation. Point the machine at something with a GPU on your LAN, or at a provider.

### An app I expected is not there

Nothing is shelved in this build. Music and ySheets were — deliberately left out because there
was nothing behind their screens — and both came back in October 2026. If an app is shelved
again, the shell says so when you ask for it, with the reason and what would bring it back.
[`design/shelved-2026-09-20.md`](../design/shelved-2026-09-20.md) is the full account.

### Reading the logs

```bash
tail -f /opt/yantrik/logs/yantrik-os.log
grep -i error /opt/yantrik/logs/yantrik-os.log
cat /opt/yantrik/BUILD          # exactly which build this machine is running
```

`/opt/yantrik/BUILD` is the answer to "what version is this" — the file the updater reads and
writes. Quote it in a bug report.

---

## Next

- **[hardware-requirements.md](hardware-requirements.md)** — what it runs on, with the
  numbers that were actually measured
- **[harness.md](harness.md)** — attaching a different mind
- **[app-control.md](app-control.md)** — how apps publish state and accept actions
- **[surface-protocol.md](surface-protocol.md)** — the protocol a surface keeps, and `yos check`
- **[architecture.md](architecture.md)** — the system design
- **[footprint.md](footprint.md)** — what it costs to run, measured
- **Issues**: <https://github.com/yantrikos/yantrik-os/issues>
