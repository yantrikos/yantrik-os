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
app, screen by screen — are in [`design/`](../design/) in this repository, written down at
the time rather than after somebody complained. Read
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

Piped straight into a shell with no flag it explains and does nothing else — it writes no
file, installs nothing and wants no privilege. That is deliberate: an installer that runs the
moment it is piped into `sh` is asking you to trust bytes you have not read.

The image is **1.84 GiB** at v0.1.0-2409 (8 Nov 2026), and will be about 0.5 GiB larger once
Hermes is built into it (PR #1169); `bytes` in `latest.json` is the exact size of the build you
are about to fetch. If you want to keep the exact build you tested, keep the file:
the `latest` name will mean the next build tomorrow.

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
not boot it, but it has been run by hand: on 9 Nov the installer wrote a UEFI install (EFI
system partition, encrypted root) to a Proxmox VM with OVMF firmware, and the installed machine
booted. Real hardware of any kind is not tested at all.

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
| Firmware | BIOS (SeaBIOS) in CI and on the project's test VM. UEFI also works: installed and booted on Proxmox OVMF (9 Nov) |

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
boot media with **4 GB of RAM and EFI enabled**. (`build-vbox-image.sh` builds an Alpine-era VM
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

The boot menu has four entries:

| Entry | What it does |
|---|---|
| **Install Yantrik OS** | Boots live *and* puts first-run setup into installer mode, so it offers to write to a disk at the end |
| **Try Yantrik OS without installing** | Boots live. Nothing offers to touch a disk |
| **Install Yantrik OS with safe graphics** | The same as the first, with `nomodeset` — use this if the screen stays black |
| **Try Yantrik OS, showing every boot message** | No `quiet`, so the kernel and systemd say what they are doing (on the serial line too). This is the entry to boot from when you are filing a bug |

Either way the machine boots to a live desktop first. Nothing is written to any disk until
you tell the installer to erase one.

Two things about the live session that are true and worth knowing before you put it on a
network:

- The live user is `yantrik` with the password `yantrik` and passwordless sudo. On a live
  image, anyone with physical access is root anyway. The disk installer asks you for a real
  password.
- SSH is installed and **disabled**. It is not turned on by any boot entry.

### What you see first

Setup happens in two parts, one on each side of the install.

**The installer** (the "Install" entries) is four screens, then the install:

1. **Welcome**, where you pick the keyboard layout.
2. **About you**: your name, a username and a password. The computer's name is worked out from
   the username and sits behind a small **More** link if you want to change it.
3. **Where to install**: the disk, and **encryption, which is on by default** ("Encrypt with my
   password"). With it on, the password is asked for every time the computer starts, and
   without it nobody can read the disk, you included: a forgotten password means an unreadable
   disk. Untick it and the screen says anyone who takes the disk out can read everything.
4. **Ready to install**: every answer and the disk together, before anything is written.

Then it installs (about six minutes on the test VM) and asks you to restart. **Take the ISO out
first** (detach it in a VM): on the 2409 nightly a machine that still has it attached boots the
installer again.

**The first run** is on the installed machine's first boot, after you unlock it with the
password you chose: a welcome, **who should do the thinking** (the AI provider, the one that
matters — see below), **what your minds share** (a screen about what other Minds may know of
you, with the provider named), then a readiness check of the minds. It is optional and
skippable; the desktop works without a model.

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
| `Super`+`Shift`+`R` | Start or stop a screen recording (saved in `~/Videos/Recordings`) |

---

## 5. Attach a mind

**The image ships no language model and no mind.** That is deliberate and it is the thing
people are most surprised by. The desktop is the desktop; what answers in it is something you
point it at.

The default configuration (`/opt/yantrik/config.yaml`) expects an OpenAI-compatible endpoint
on `http://127.0.0.1:8341/v1` — loopback, because an address baked into a public image is an
address every copy of that image would talk to. First-run setup is where you change it, and
you have three shapes of answer:

1. **A model you run yourself.** Ollama or llama.cpp on the same machine or elsewhere on
   your LAN. Nothing leaves your network.
2. **A provider you choose.** Any OpenAI-compatible endpoint, or the Claude CLI. What you
   type goes to that provider, as it would from any other client.
3. **A separate agent that attaches to the desktop.** Yantrik Mind, Hermes, Pi, DeepSeek and
   OpenClaw all do this; the last four ship as source in [`harnesses/`](../harnesses/). The
   OS does not hold the harness's endpoint, model name or key — the harness dials in and keeps its
   own configuration (the one exception is "use a saved provider" in Settings, which copies a key
   from the credential vault into that harness's settings, and can be reverted). See **[harness.md](harness.md)** for the protocol and how to write one.

Whichever you pick, the desktop says which mind is answering: the Lens's conversation names it,
and the Mind panel (hidden until you pin it, for instance with `yos act shell set_mind_panel shape=pinned`) shows
"Answering: <mind>" and what it runs on, the provider and model, for instance "Ollama Cloud ·
deepseek-v4.1-flash". The top bar does **not** carry a chip for this on the 2409 nightly; a
single "answering · account · model" line used everywhere is in review (PR #1162). The point of
all of it is that "where is what I am typing going" is never a question you have to go and
look up.

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
| `bypass` | nothing is asked except before anything an app marks as impossible to undo, such as a purchase or a calendar delete — that still puts a card up — and once per session before running commands. Time-boxed, and never written to disk, so no machine boots into it |
| `bypass_all` | "Full bypass": nothing is asked at all, those included, with one exception: once a mind connected to this desktop has read your private data, its command, hand-off to another mind, new web address or send of text off this machine still puts a card up. The built-in assistant's own mail is not covered: in `bypass_all` it sends without one. Time-boxed and never written to disk, like `bypass`; chosen on the same confirmation |

A command — the Terminal's `run`, an agent's own terminal — can do anything you can, so it asks
before it runs in `ask`, `auto` and `bypass`. Press **Allow for this session** on that card and
the mind's commands after it run without asking until the shell restarts.

Everything that ran without you being asked is written down in
`~/.local/share/yantrik/mind-audit.jsonl` and readable from the mode menu.

### Stopping a mind

Type **`/stop`** in the Lens: it is never queued as a message, it goes straight to the running
turn and ends it, and closes any question the mind was waiting on. On the 2409 nightly there is
**no Stop button**, and a Hermes run that is already working may carry on after the desktop has
stopped waiting for it. A Stop button (PR #1072) and Hermes honouring Stop (PR #1161) are in
review; remove this paragraph's caveats when they merge.

---

## 6. Install it to a disk

**This erases the disk you point it at.** Use a machine, or a VM, you can afford to wipe. The
disk installer has not been through the boot test that the image itself has — treat it as the
least-proven part of this.

The target disk needs at least **6 GB** — one minimum, stated here, in the README's hardware
table, and in the first-boot hardware scan, which a test holds together. The scan measures the
whole size of the disk the picker has selected; it used to report the free space of the live
session, which on a live USB describes the stick you booted from and not the disk you are
about to install to.

Two ways in, and they do the same work:

- **The graphical installer.** Boot the "Install Yantrik OS" entry: the four screens described
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

It asks for your name, a username, a password, a hostname and a timezone (guessed from the
network, one keystroke to accept), then shows you every answer and the disk together and
waits for you to type `yes`. Then it partitions (GPT, EFI or BIOS depending on how you
booted), copies the live filesystem to the disk, removes the live-boot packages, creates your
account, installs GRUB and reboots.

After that the machine boots to the desktop with no display manager, and the shell starts
locked: unlock it with your password. (If you left encryption on, that password is also asked
at the very start, to open the disk.) The first run described above happens once, on that first
boot.

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
the renderer settings are in `~/.config/labwc/environment` —
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
