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
- **Any GPU other than QEMU's virtio-vga.** The desktop now uses NVIDIA, AMD and Intel GPUs
  when Mesa reports them (see [GPU](#gpu)), and that path has not been run on real hardware
  by this project yet.
- **Wi-Fi on real adapters.**
- **VirtualBox.** The image boots, but its VMSVGA 3D cannot be used by the desktop; it draws in
  software there (see [VirtualBox](#virtualbox)).

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

The desktop does not need one, and uses one when it has one that works.

At every login, before the compositor starts, `yantrik-session` decides between the GPU and
software rendering, and the compositor (labwc), the shell and every app follow that one
decision. The rule, first match wins:

1. **A person's choice.** `nomodeset` on the kernel command line (the **Safe Mode** boot entry)
   means software. So does `yantrik.graphics=software` on the kernel command line, and
   `yantrik.graphics=gpu` means the GPU. After those, `YANTRIK_GRAPHICS=gpu|software`,
   `WLR_RENDERER=…` or `LIBGL_ALWAYS_SOFTWARE=1` in `~/.config/labwc/environment` or in the
   session's environment. A `WLR_RENDERER` or `SLINT_BACKEND` you set is never overwritten.
2. **Ask Mesa.** `eglinfo -B -p gbm`, with a 5-second timeout, and read the OpenGL ES
   renderer. llvmpipe, softpipe, swrast, kms_swrast, or no answer means software.
3. **Combinations known to be broken**, whatever the renderer string says: **vmwgfx on a
   hypervisor that is not VMware** (VirtualBox, below), and **virtio-gpu without virgl**.
4. **The GPU failed on this machine before**, for this renderer string and this build.
5. Otherwise, **the GPU**: labwc on its default renderer, the shell on femtovg, and nothing sets
   `LIBGL_ALWAYS_SOFTWARE`.

A GPU chosen by rule 5 is a trial. If the compositor or the shell dies within 45 seconds of
starting, the session writes `~/.local/state/yantrik/graphics-fallback` (the reason, the
renderer string, the build), starts the desktop again in software straight away, and shows a
notification saying so. It does not try that GPU again until the renderer string or the
installed build changes. Delete the file to try sooner.

`yantrik-session graphics` prints what this machine would decide and why. What it did decide
is in `yos describe shell` (`graphics`) and in Settings → About.

### VirtualBox

Found on 2026-09-23 with VirtualBox's VMSVGA adapter, 3D enabled, Debian 13, kernel 6.12:
Mesa accelerates it (`eglinfo` reports `SVGA3D; build: RELEASE; LLVM;`), and the desktop
still cannot use it. With labwc on its GL renderer, labwc fails with "importing the supplied
dmabufs failed" on the shell's first frame (with or without `WLR_DRM_NO_MODIFIERS=1`), and the
kernel logs that vmwgfx "seems to be running on an unsupported hypervisor". So on VirtualBox
the desktop draws in software whatever the 3D setting says. This is also why rule 5 is a trial
rather than a promise: a renderer string that looks like hardware is not proof that the
compositor can use it.

To watch the fallback do its job on a machine like this, boot once with `yantrik.graphics=trial`
on the kernel command line (or put `YANTRIK_GRAPHICS=trial` in `~/.config/labwc/environment`).
That lifts rule 3 alone: the GPU is tried, the first start fails, the session records it and
comes back in software, and the notification and Settings → About say why. A record already on
the machine still wins — delete `~/.local/state/yantrik/graphics-fallback` to watch it again.

### Why software is the fallback and not software OpenGL

Measured on WSLg, the shell on its animated desktop: **41 %** of a core on the GPU,
**98 %** with Slint's own software rasteriser, and **576 %** — six cores — when it is pointed
at OpenGL and Mesa answers with llvmpipe. Guessing "GPU" on a machine without one is the worst
of the three, so every unknown lands on software. On software the shell also turns its ambient
animation off, because a full-screen software frame costs about 96 ms however rarely it is
asked for.

Where a GPU matters beyond that is **inference**, and only if you choose to run the model on
this machine. See below.

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
