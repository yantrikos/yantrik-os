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
[`design/shelved-2026-09-20.md`](../design/shelved-2026-09-20.md) for two apps that were
taken out of the build for being drawings of apps.

---

## 1. Download the image

The current image and its checksum are published at
**<https://iso.yantrikos.com/nightly/>**, with `latest.json` naming the current file, its
sha256, its size and its version. Four lines, and you can see every step:

```bash
curl -fsSL https://iso.yantrikos.com/nightly/latest.json          # what the current file is called
curl -fL -O https://iso.yantrikos.com/nightly/<that file>
curl -fL -O https://iso.yantrikos.com/nightly/<that file>.sha256
sha256sum -c <that file>.sha256
```

`install.sh` in this repository does the same thing for you and is also served from
`https://get.yantrikos.com/install.sh`:

```bash
sh install.sh               # prints the url, size and sha256 of the current image, and stops
sh install.sh --download    # also fetches it here and checks its sha256
```

Piped straight into a shell with no flag it explains and does nothing else — it writes no
file, installs nothing and wants no privilege. That is deliberate: an installer that runs the
moment it is piped into `sh` is asking you to trust bytes you have not read.

The image is about **1.3 GiB**. There is also `yantrik-os-nightly-latest.iso`, which always
points at the newest one — convenient for a script, less good for a person, because the file
you tested is not the file that name will mean tomorrow.

**Verify the checksum before you boot it.** The point is not that someone is attacking you;
it is that a 1.3 GiB download that ends at 98% gives you an image that boots halfway and
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

That test is a BIOS boot under QEMU with a virtio display. The UEFI path is built and is not
booted by CI; real hardware of any kind is not tested at all.

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
| GPU | none — virtio-gpu without virgl, so the desktop draws in software |
| Firmware | BIOS (SeaBIOS). The UEFI path is built into the image and is untested |

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
| **Try Yantrik OS (live, no install)** | Boots live. Nothing offers to touch a disk |
| **Install Yantrik OS (Safe Mode)** | The same as the first, with `nomodeset` — use this if the screen stays black |
| **Try Yantrik OS (verbose, serial console)** | No `quiet`, so the kernel and systemd say what they are doing. This is the entry to boot from when you are filing a bug |

Either way the machine boots to a live desktop first. Nothing is written to any disk until
you tell the installer to erase one.

Two things about the live session that are true and worth knowing before you put it on a
network:

- The live user is `yantrik` with the password `yantrik` and passwordless sudo. On a live
  image, anyone with physical access is root anyway. The disk installer asks you for a real
  password.
- SSH is installed and **disabled**. It is not turned on by any boot entry.

### What you see first

First-run setup, as a sequence of full-screen questions: your name, what you are interested
in, a hardware scan, how the assistant should behave, and then **the AI provider**, which is
the one that matters — see below. If you booted an "Install" entry, the last screen also
offers to install to disk.

Behind it is the shell: a status bar, a dock, and the Lens at the bottom, which is where you
type to the mind.

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
| `Print` / `Super`+`Shift`+`S` | Screenshot: whole screen / a region, into `~/Pictures` |

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
   OS never holds your endpoint, model name or key — the harness dials in and keeps its own
   configuration. See **[harness.md](harness.md)** for the protocol and how to write one.

Whichever you pick, the status bar says which mind is answering and whether it is local or
cloud, with the provider or model named. That chip is not decoration: it is there so that
"where is what I am typing going" is never a question you have to go and look up.

### How much it may do without asking

Also in the status bar, to the left of the mind chip, is the **mode**:

| mode | the mind may… |
|---|---|
| `plan` | read only — every change is refused and it has to tell you what it *would* do |
| `ask` | routine things run; sensitive ones put a card in front of you (the default) |
| `auto` | sensitive things run; you are still asked about destructive ones |
| `bypass` | nothing is asked. Time-boxed, and never written to disk, so no machine boots into it |

Everything that ran without you being asked is written down in
`~/.local/share/yantrik/mind-audit.jsonl` and readable from the mode menu.

---

## 6. Install it to a disk

**This erases the disk you point it at.** Use a machine, or a VM, you can afford to wipe. The
disk installer has not been through the boot test that the image itself has — treat it as the
least-proven part of this.

Two ways in, and they do the same work:

- **The graphical installer.** Boot the "Install Yantrik OS" entry and first-run setup ends
  with the install screen — username, password, hostname, disk.
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

After that the machine boots straight to the desktop with no login screen and no display
manager, and onboarding does not run again.

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

`apply` takes `--force` (reinstall the same build), `--reboot` (instead of restarting the
session) and `--no-restart`. `check` and `status` take `--porcelain`, which is what the
desktop's About screen reads.

What it does for you: the bundle is verified against the manifest's sha256 before a single
file is replaced, the current binaries and shared files are backed up first, and if the new
shell does not answer its control socket after the restart the update rolls itself back. The
restart goes through the session unit rather than respawning the shell by hand, so the
desktop comes back exactly as it does on boot.

Again: `nightly` is the only channel with builds on it. `set-channel stable` will succeed and
then `check` will tell you the channel is not published, which is the honest answer rather
than an error.

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

Boot the **Safe Mode** entry, which adds `nomodeset`: no GPU driver loads, and the desktop
draws in software.

The desktop chooses between the GPU and software by itself at every login, and when a GPU
fails in its first 45 seconds it starts again in software and says so — see
[GPU](hardware-requirements.md#gpu) for the whole rule. To see what it decided and why:

```bash
yos describe shell | grep -A12 '"graphics"'   # what the shell draws with, and who decided
/opt/yantrik/bin/yantrik-session graphics      # what this machine would decide right now
cat ~/.local/state/yantrik/graphics-fallback   # present if a GPU failed here and was given up on
```

### Forcing the GPU or software

Add one line to `~/.config/labwc/environment` and log out (or reboot):

```bash
YANTRIK_GRAPHICS=software    # compositor, shell and apps all draw on the CPU
YANTRIK_GRAPHICS=gpu         # the GPU, even where the probe or an earlier failure says no
```

At the boot menu, `yantrik.graphics=software` or `yantrik.graphics=gpu` on the kernel command
line does the same for one boot, which is the way in when the desktop will not start at all.
`WLR_RENDERER` (the compositor's renderer) and `SLINT_BACKEND` (the shell's, e.g.
`winit-software`) are honoured as you write them. A forced GPU is never undone by the automatic
fallback: if it does not work, boot with `yantrik.graphics=software` and remove the line.

To let a machine that fell back try its GPU again before the next update, delete
`~/.local/state/yantrik/graphics-fallback`.

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

Music and ySheets are **shelved** — deliberately not in this build, because there was nothing
behind their screens. The shell says so when you ask for one, with the reason and what would
bring it back. [`design/shelved-2026-09-20.md`](../design/shelved-2026-09-20.md) is the full
account.

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
