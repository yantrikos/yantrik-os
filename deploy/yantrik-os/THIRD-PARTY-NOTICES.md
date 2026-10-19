# Third-party software in a Yantrik OS image

Installed to `/opt/yantrik/THIRD-PARTY-NOTICES.md` by `build-debian-iso.sh`.

This lists what a Yantrik OS ISO redistributes that Yantrik OS did not write. It exists
because publishing an image is redistribution, and several of the things below are licensed
on the condition that their terms travel with the copy.

**Status: incomplete.** Every entry marked `NEEDS VERIFICATION` has a licence that was not
read from the upstream artifact at build time. They are marked rather than guessed, because a
guessed licence in an attribution file is worse than an absent one — it is a false claim about
someone else's terms. Clear these before the image is published.

---

## The Debian base system

The image is built with `debootstrap` from Debian 13 (trixie) plus packages installed from
`deb.debian.org`, including `contrib`, `non-free` and `non-free-firmware`.

Each package carries its own licence at `/usr/share/doc/<package>/copyright` **inside the
image** — that is the authoritative list, it is complete, and it ships. Notable components:

| Component | Licence |
|---|---|
| Linux kernel (`linux-image-amd64`) | GPL-2.0 |
| systemd, NetworkManager, udisks2 | LGPL-2.1+ / GPL-2.0+ |
| labwc (the compositor) | GPL-2.0 |
| foot (the terminal) | MIT |
| Chromium | BSD-3-Clause and others (see its own `copyright`) |
| Mesa, PipeWire, WirePlumber | MIT / LGPL-2.1+ |
| mako, grim, slurp, wl-clipboard, swaybg | MIT / ISC |
| wf-recorder (screen recording) | MIT |
| vainfo (libva-utils: whether the GPU encodes H.264) | MIT |
| `firmware-*` (non-free) | Redistributable binary firmware, vendor terms — see each `copyright` |

**GPL source offer.** The kernel and much of the base are GPL. Redistributing Debian's
unmodified binary packages is covered by pointing at Debian's source for the same versions
(GPL-2 §3(c) / GPL-3 §6(d)); the image does not modify them. A published ISO needs a written
offer or a link saying so. **This offer is not yet written — see the publish checklist.**

## Models and voice

| Artifact | Source | Licence |
|---|---|---|
| `all-MiniLM-L6-v2` (embedder, `/opt/yantrik/models/embedder`) | `huggingface.co/sentence-transformers/all-MiniLM-L6-v2` | Apache-2.0 |
| `whisper-tiny` (speech to text, `/opt/yantrik/models/whisper`) | `huggingface.co/openai/whisper-tiny` | Apache-2.0 |
| Piper (`/opt/yantrik/models/tts/piper`) | `github.com/rhasspy/piper` release 2023.11.14-2 | MIT |
| espeak-ng data bundled inside the Piper release | `github.com/espeak-ng/espeak-ng` | GPL-3.0 — **NEEDS VERIFICATION**: GPL data shipped beside Piper's MIT binary has its own source-offer obligation |
| `en_US-lessac-medium` voice | `huggingface.co/rhasspy/piper-voices` v1.0.0 | **NEEDS VERIFICATION** — read `MODEL_CARD` beside the `.onnx` in that repo and transcribe it here verbatim |
| Offline LLM (`--with-llm` builds only) | Whatever `LOCAL_GGUF` pointed at, or `unsloth/Qwen3.5-4B-GGUF` | **NEEDS VERIFICATION** — a `--with-llm` image must name the exact model and its licence. Qwen models ship under their own community licence, not Apache-2.0, and a locally fine-tuned GGUF inherits the base model's terms |

An image built **without** `--with-llm` ships no LLM weights. The default build is that one.

## Fonts

| Font | Licence |
|---|---|
| Barlow (3 weights, `/opt/yantrik/share/fonts`, from `crates/yantrik-design-tokens`) | SIL Open Font License 1.1 — **NEEDS VERIFICATION**: the `OFL.txt` must ship beside the `.ttf` files, and currently does not |
| JetBrains Mono (2 weights, same directory) | SIL Open Font License 1.1 — **NEEDS VERIFICATION**: same missing `OFL.txt` |
| DejaVu (`fonts-dejavu-core`) | Bitstream Vera / public domain — ships in the package's own `copyright` |

Both font families are also *embedded in each app binary* by Slint, not only shipped loose.

## Yantrik OS itself

The binaries under `/opt/yantrik/bin` are Yantrik OS, licensed **GPL-3.0** (`LICENSE` in the
repository, installed to `/opt/yantrik/LICENSE`; `README.md` says the same).

That licence has a consequence for a published ISO that nothing in the build currently meets:
GPL-3 §6 requires that whoever receives the binaries can get the **corresponding source** for
the exact version they received. A download page for the ISO therefore has to carry either the
source alongside it or a written offer naming where to get it, pinned to the git revision in
`/opt/yantrik/BUILD`. **Not yet written.**

Note also that the workspace `Cargo.toml` declares no `license` field, so `cargo` metadata and
anything generated from it will report the licence as unknown.

Rust dependencies compiled into those binaries (predominantly MIT/Apache-2.0) are not
enumerated here. `cargo about` or `cargo deny` generates that list from `Cargo.lock`; it is
not yet wired into the build.
