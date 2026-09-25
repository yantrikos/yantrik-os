# Hardware requirements

Yantrik OS is a **Debian 13 (trixie)** live image for **x86-64**. There is no ARM build and
no 32-bit build.

Every number on this page is one of three things, and each is labelled: **measured** (someone
ran the command and this is what it said), **configured** (a value written in a file in this
repository, which you can go and read), or **not measured** (nobody has checked, so no figure
is given). A requirements page whose numbers came from nowhere is how this OS spent months
telling people to install Alpine Linux.

---

## The image

| | |
|---|---|
| Size | **1,411,915,776 bytes — about 1.31 GiB** (measured: `bytes` in `https://iso.yantrikos.com/nightly/latest.json`, build `v0.1.0-304`, 2026-09-22) |
| Format | Hybrid BIOS + EFI ISO, `grub-mkrescue`. Write it to a USB stick or attach it to a VM |
| Channel | **nightly only.** `beta` and `stable` are names the updater knows; nothing has ever been published to either |

The size moves slightly from build to build. `latest.json` is always the current answer —
that is what it is for.

---

## What it has actually been run on

### The automated boot test — the one thing every published image has passed

[`.github/workflows/iso.yml`](../.github/workflows/iso.yml) boots each image before it is
published, and refuses to publish one that does not come up. It uses
(configured, [`boottest.py`](../deploy/yantrik-os/boottest.py)):

| | |
|---|---|
| RAM | **4096 MB** |
| CPUs | **4** |
| Display | `-device virtio-vga`, headless (`-display none`), read over the serial console |
| Firmware | BIOS (QEMU's default) |
| Acceleration | KVM when the runner has `/dev/kvm`, plain emulation when it does not — both are exercised |
| Disk | none. The test never installs; it boots the live image |

So: **4 GB of RAM, 4 CPUs, no GPU, no disk** is a configuration the current image is known to
boot in, because it did.

### The project's test machine

The VM this is developed and demonstrated against (measured, 2026-09-22, over SSH):

| | |
|---|---|
| Platform | QEMU/KVM — `Standard PC (Q35 + ICH9, 2009)`, SeaBIOS |
| CPU | 4 cores (`nproc`) |
| RAM | 7.8 GiB total (`free -h`) — provisioned as 8 GB |
| GPU | none. `Red Hat, Inc. Virtio 1.0 GPU` — software rendering through Mesa's llvmpipe |
| Disk | 32 GB, of which 17 GB used |
| Kernel | `6.12.107+deb13-amd64` |
| Firmware | BIOS. `/sys/firmware/efi` does not exist on it |

That is the whole of what this OS is known to run well on. It is a VM with no GPU, and the
desktop is fully usable on it.

### Everything else

**Not measured.** Specifically:

- **Real hardware of any kind.** No published image has been booted on a physical machine by
  this project.
- **The UEFI boot path.** It is built into every image and CI boots the BIOS path.
- **Any GPU other than QEMU's virtio-vga.** NVIDIA, AMD and Intel hardware acceleration are
  untested here.
- **Wi-Fi on real adapters.**
- **VirtualBox.** It should work — the image is an ordinary Debian live ISO — but nobody has
  checked it against a current build. The settings this repository's own tools give it
  (VMSVGA, 128 MB of video memory, EFI, 4 GB of RAM) are configured, not measured, and are
  written out in [getting-started.md](getting-started.md).

---

## What to give a VM

Start from what is known to work rather than from a guess:

| | Known-good | Minimum anyone has booted |
|---|---|---|
| CPU | 4 vCPU | 4 vCPU (CI) |
| RAM | 8 GB | 4 GB (CI) |
| Disk | 32 GB | none, if you only boot live |
| GPU | none needed | none needed |
| Firmware | BIOS | BIOS |

Below 4 GB is **not measured** — it may well work, and no one has tried it, so this page is
not going to print a number.

```bash
qemu-system-x86_64 -enable-kvm -m 4096 -smp 4 \
  -cdrom yantrik-os-<version>.iso -boot d \
  -device virtio-vga -display gtk
```

---

## Disk, if you install it

The minimum target disk is **6 GB** (configured: `MIN_DISK_BYTES` in the first-boot hardware
scan; the README's hardware table carries the same number and a test holds the two together).
The scan measures the whole size of the disk the installer's picker has selected, with
`lsblk -b` — it used to report the free space of the live session, which on a live USB
describes the stick you booted from and not the disk you are installing to.

The image copies its own live filesystem onto the disk, so an installed machine is roughly a
Debian 13 system plus `/opt/yantrik`.

`/opt/yantrik` on the test machine, measured 2026-09-22 with `du -sh`:

| Directory | Size | |
|---|---|---|
| `bin/` | **757 MB** | the shell, the apps, the services, `yos`, `yantrik-update`, `yantrik-install`. Includes ~112 MB of duplicate binaries left by hand-deploys on this particular machine — a fresh install is smaller |
| `models/` | **354 MB** | whisper 147 MB (speech to text), tts 120 MB (the voice), embedder 88 MB (memory search). `llm/` is empty — **no language model ships** |
| `data/` | **64 MB** | the memory database. Grows with use |
| `logs/` | 9.3 MB | |
| `share/` | 616 KB | compositor config, window theme, typeface, the `.desktop` entries |
| `skills/`, `i18n/`, config | ~370 KB | |
| | **≈ 1.2 GB** | for those directories together, on this machine |

Two things that machine also has and a fresh install does not: `backups/` (2.0 GB — three
rollback copies from three updates; `yantrik-update` prunes to the newest two at the start of
the next one) and `ui-deployments/` (6.5 GB of development artefacts). Its whole
`/opt/yantrik` is 9.6 GB; that number describes a development machine, not an install.

**A fresh installed footprint has not been measured.** From the image size and the directory
breakdown above, `/opt/yantrik` should land somewhere around 1 GB, plus the Debian base. The
32 GB the test machine has is comfortable. `yantrik-update` refuses to start an update with
less than **1,500,000 KB — about 1.4 GiB — free** on `/opt/yantrik` (configured, `need_kb` in
`yantrik-update`), because it writes a full backup of `bin/` and `share/` before it replaces
anything, and keeps two.

---

## GPU

The desktop does not need one. The shell renders through Slint's software rasteriser and the
compositor through Mesa's llvmpipe; that is how the test machine and the CI boot test both
run, and it is a deliberate trade rather than a fallback.

Where a GPU matters is **inference**, and only if you choose to run the model on this machine.
See below.

---

## The model, and what this machine needs for it

**The image ships no language model and no mind.** This is the fact that changes what hardware
you need more than any other, and it is easy to miss.

- If you point the desktop at **a model on another machine** (Ollama on your LAN) or at **a
  provider**, this machine does no inference at all and the requirements above are the whole
  story.
- If you want to run a model **on this machine**, its requirements are the model's, not this
  OS's, and they are additional to everything above. A GPU makes the difference between a
  conversation and a wait.

This page deliberately prints no tokens-per-second table. The one that used to be here gave
four models, four speeds and four quality grades with no statement of what hardware produced
any of them, and there is no measurement in this repository behind those numbers.

---

## Network

- **Downloading the image** needs internet. About 1.31 GiB.
- **Running it** does not, in itself. Nothing in the OS calls out on its own.
- **Whatever mind you attach** needs whatever it needs: nothing for a local model, LAN access
  for an Ollama box, internet for a provider.
- **Updating** needs to reach `releases.yantrikos.com` over HTTPS.
- Weather, email and calendar reach the services you configure them against.

---

## Memory at runtime

Measured 2026-09-06 (WSL2, software renderer, shell plus three autostarted services):
**about 186 MB PSS** for the whole desktop, of which the shell is 181 MB RSS — 87 MB of that
is the embedder's weights.

[footprint.md](footprint.md) has the full measurement, how it was taken, and the two traps in
taking it. Note the warning there: **never quote a memory figure from a `fast` build** — it is
about a third too high.

Inference memory is the model's, not the desktop's, and is not included in that figure.
