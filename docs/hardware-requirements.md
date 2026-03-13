# Hardware Requirements

## Minimum Requirements

| Component | Requirement |
|-----------|-------------|
| **CPU** | x86_64, 2 cores |
| **RAM** | 4 GB |
| **Disk** | 4 GB free space |
| **GPU** | Not required (software rendering works) |
| **Display** | 1024x768 minimum |
| **OS** | Alpine Linux 3.18+ |

With minimum specs, the system runs with software rendering. The bundled Qwen 3.5 4B model provides reliable reasoning and structured output, but **a GPU is strongly recommended** for interactive AI responses. On CPU-only, inference is too slow (~1 TPS) for practical use — the companion becomes a background assistant rather than an interactive one.

## Recommended Specs

| Component | Recommendation |
|-----------|----------------|
| **CPU** | 4+ cores, modern x86_64 |
| **RAM** | 8+ GB |
| **Disk** | 5+ GB free (10+ GB if hosting LLM models locally) |
| **GPU** | **NVIDIA or AMD strongly recommended** for usable AI response times |
| **Display** | 1920x1080 |
| **OS** | Alpine Linux 3.23 |

> **GPU is essential for interactive AI.** Without GPU acceleration, even the smallest models produce ~1 token/second on CPU, making real-time conversation impractical. With a GPU (via Ollama), the 4B model runs at 70+ TPS — fully interactive. If you don't have a local GPU, point Yantrik to a remote Ollama server on your LAN that does.

## Supported Platforms

### Bare Metal

Best performance. Works with any x86_64 machine that can run Alpine Linux. GPU acceleration is automatic when drivers are available.

### VirtualBox

Fully supported. The installer auto-detects VirtualBox and:
- Installs Guest Additions
- Enables software rendering (`SLINT_BACKEND=winit-software`)
- Disables hardware cursors

**Recommended VM settings:**
- 2+ CPU cores
- 2048+ MB RAM
- 32 MB video memory
- VBoxVGA or VBoxSVGA display adapter
- Enable 3D acceleration if available

### QEMU/KVM

Fully supported. The installer configures:
- `WLR_DRM_DEVICES=/dev/dri/card0`
- `SLINT_BACKEND=winit`

**Recommended QEMU flags:**
```bash
qemu-system-x86_64 \
  -enable-kvm \
  -m 2048 \
  -smp 2 \
  -display gtk,gl=on \
  -device virtio-vga-gl
```

### Proxmox LXC/VM

Works in both LXC containers and full VMs. For LXC, pass through `/dev/dri` for GPU access.

## GPU Support

### No GPU (CPU Only)

The UI works fine with software rendering via Mesa's llvmpipe. However, **AI inference on CPU-only is extremely slow** (~1 TPS for even the smallest models). The companion still functions but responses take 30-60+ seconds. For usable AI, point to a remote Ollama server with GPU access on your LAN, or use a cloud backend (Claude CLI, OpenAI API).

### NVIDIA

For GPU-accelerated AI inference:
1. Install NVIDIA drivers on the host (not in the VM)
2. Run Ollama on the host with GPU access
3. Point Yantrik's LLM config to the Ollama server

The Yantrik UI itself runs on Wayland via labwc and doesn't need NVIDIA drivers for rendering.

### AMD / Intel

Mesa drivers are installed automatically. Hardware-accelerated rendering works out of the box on most AMD and Intel GPUs.

## AI Model & Backend Options

The installer bundles Qwen 3.5 4B Q4_K_M (~2.5 GB) for offline capability. For interactive use, a GPU backend is required:

| Setup | Model | Speed | Quality |
|-------|-------|-------|---------|
| **Ollama with GPU (recommended)** | Qwen 3.5 4B–27B+ | 50-130 TPS | Strong to excellent |
| **Remote Ollama on LAN** | Any model | Depends on server | Best flexibility |
| **Cloud backend** | Claude, GPT, etc. | Network-bound | Excellent |
| **CPU-only (bundled model)** | Qwen 3.5 4B Q4 | ~1 TPS | Strong quality, but too slow for interactive use |

All small models run with thinking disabled (`think: false`) to maximize output quality. The model-adaptive intelligence system automatically adjusts tool exposure, prompt strategy, and agent behavior based on the detected model size.

### Model quality benchmarks (Qwen 3.5 family)

| Model | Reasoning | Structured Output | Format Following | Speed (GPU) |
|-------|-----------|-------------------|-------------------|-------------|
| 0.8B | Fails | CSV/JSON work | Slide format errors | ~105 TPS |
| 2B | Fails | Good | Good | ~81 TPS |
| **4B** | **Correct** | **Perfect** | **Good** | **~71 TPS** |
| 9B | Correct | Perfect | Best | ~55 TPS |

The 4B model is the minimum viable size — it's the smallest that handles multi-step reasoning (scheduling, math) correctly.

## Disk Space Breakdown

| Component | Size |
|-----------|------|
| Yantrik binaries (`yantrik-ui` + `yantrik`) | ~50 MB |
| MiniLM embedder model | ~87 MB |
| Whisper voice model | ~146 MB |
| System packages (labwc, mesa, fonts, etc.) | ~200 MB |
| Offline LLM (Qwen 3.5 4B Q4, included) | ~2.5 GB |
| Memory database (grows over time) | starts at ~1 MB |
| **Total** | **~3 GB** |

If using a remote Ollama server or cloud LLM, you can skip the offline LLM model — saving ~2.5 GB.

## Network Requirements

- **Installation**: Internet access required for downloading packages and models
- **Runtime**: Internet optional — depends on your LLM backend choice:
  - Cloud backends (Claude CLI, OpenAI API): Requires internet
  - Local Ollama: Requires LAN access to Ollama host (or localhost)
  - Built-in fallback: No network needed (fully offline)

The companion's email, weather, and calendar features naturally require network access.

## Performance Notes

- **UI rendering**: ~60 FPS on bare metal with GPU, ~30 FPS in VirtualBox with software rendering
- **LLM inference**: Depends entirely on your backend. CPU-only: ~1 TPS (not interactive). Ollama with GPU: 50-130+ TPS (fully interactive). Cloud: depends on network latency.
- **Memory footprint**: ~150 MB RSS at idle, ~500 MB–1.5 GB during active AI inference (with built-in 4B model)
- **Startup time**: ~3-5 seconds to desktop on SSD, ~10 seconds on HDD
