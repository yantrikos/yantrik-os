# Session Handoff — YOS-414: Notifications & Awareness

## What's Done

All YOS-414 code is **implemented, built, and deployed** to the VBox VM. The 9 changed files have been copied from the OneDrive repo to this clean clone.

### Changes ready to commit (in this repo):

| File | Change | Task |
|------|--------|------|
| `crates/yantrik-ui/src/features/notification_relay.rs` | **Rewritten**: per-app urgency learning, 3+ notification grouping per app within 60s, feedback adaptation (Acted → score up, Dismissed → score down) | YOS-415 |
| `crates/yantrik-ui/src/features/resource_guardian.rs` | Enhanced narratives: battery with clock time projection, memory with top 3 processes + tool hints, disk with analyze_disk hint | YOS-416/417/418 |
| `crates/yantrik-ui/src/features/network_watcher.rs` | **NEW**: NetworkWatcher feature — disconnect/reconnect/weak signal narration | YOS-419 |
| `crates/yantrik-ui/src/features/mod.rs` | Added `pub mod network_watcher;` | YOS-419 |
| `crates/yantrik-ui/src/app_context.rs` | Registered `NetworkWatcher::new()` in feature registry | YOS-419 |
| `crates/yantrikdb-companion/src/tools/process.rs` | Added `diagnose_process` tool (RSS, CPU%, threads, children, open files) | YOS-416 |
| `crates/yantrikdb-companion/src/tools/system.rs` | Added `battery_forecast` tool (wall-clock empty/full time projection) | YOS-418 |
| `crates/yantrikdb-companion/src/tools/disk.rs` | Added `analyze_disk` tool (subdir sizes, old files, file type breakdown) | YOS-417 |
| `crates/yantrikdb-companion/src/tools/networking.rs` | Added `network_diagnose` tool (DNS latency, gateway ping, internet check, WiFi) | YOS-419 |
| `crates/yantrik-ui/src/lens.rs` | Changes from prior YOS-403 commit (was unpushed from OneDrive) | YOS-403 |

### Note on lens.rs
The `lens.rs` diff includes changes from the YOS-403 "OS Shell Intelligence" work that was committed locally in the OneDrive repo (44e5f43) but pushed via a different clone (953fb06). The content may differ slightly. Include it in the commit.

## What Needs to Happen

1. **Commit and push** all changes above with message:
   ```
   Notifications & Awareness: triage, narrative urges, network watcher, diagnostic tools
   ```

2. **Close Jira tasks** (cloudId: `43533997-c827-484d-a825-e9707409e6e4`):
   - YOS-415, YOS-416, YOS-417, YOS-418, YOS-419 → transition to Done (id=31)
   - YOS-414 (epic) → transition to Done (id=31)

3. **Next epic** — the user wants to continue building "wow" features. Remaining Phase 1 epics:
   - YOS-410: Clipboard Intelligence
   - YOS-426: Voice MVP
   - YOS-431: Privacy Core

## Build & Deploy

- Build: `wsl.exe -d Ubuntu -- bash -lc "cd '/mnt/c/Users/sync/codes/yantrik-os' && CARGO_TARGET_DIR=/home/yantrik/target-yantrik RUSTFLAGS='-A warnings' cargo build --release"`
- Deploy: `bash deploy.sh` (update paths in deploy.sh if needed — currently points to OneDrive path)
- The binary was already deployed from the OneDrive repo's build. No need to rebuild unless code changes.

## Important Context

- This repo is a **clean clone outside OneDrive** at `C:\Users\sync\codes\yantrik-os`
- The original repo at `C:\Users\sync\OneDrive\Documents\GitHub\yantrik-os` has OneDrive corruption (mmap failures on git objects)
- The MEMORY.md and project memory files are at `C:\Users\sync\.claude\projects\c--Users-sync-OneDrive-Documents-GitHub-yantrik-os\memory\`
- Saga tracker project: AIDB (id=1)
- Rust 1.93.1 in WSL2 — known ICE with `check_mod_deathness`, use `RUSTFLAGS="-A warnings"`

---

## VirtualBox VM — Setup & Access

- **VM Name**: `Yantrik-OS`
- **OS**: Alpine Linux (musl), labwc Wayland compositor
- **SSH**: Port-forwarded `127.0.0.1:2222` → VM port 22
- **SSH key**: `C:\Users\sync\.ssh\id_deploy`
- **Users**: `root` (deployment), `yantrik` (UID 1000, runs desktop)

### SSH Access

```bash
# Root shell
ssh -o StrictHostKeyChecking=no -i /c/Users/sync/.ssh/id_deploy -p 2222 root@127.0.0.1

# SCP files
scp -o StrictHostKeyChecking=no -i /c/Users/sync/.ssh/id_deploy -P 2222 FILE root@127.0.0.1:/path/
```

### VM Directory Layout

```
/opt/yantrik/
├── bin/yantrik-ui       # Main desktop binary (Slint + CompanionService)
├── bin/yantrik           # CLI companion (YantrikDB + tools)
├── config.yaml           # OS config (LLM endpoint, tools, personality)
├── logs/yantrik-os.log   # Runtime logs
└── data/                 # SQLite DB, embeddings, memory
```

### Environment Variables (required for yantrik-ui)

```bash
WAYLAND_DISPLAY=wayland-0
XDG_RUNTIME_DIR=/run/user/1000
DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus
SLINT_BACKEND=winit-software    # No GPU in VM
LD_PRELOAD=/usr/lib/libgcompat_shim.so  # glibc→musl shim
```

---

## deploy.sh Workflow

The `deploy.sh` script at repo root does the full pipeline:

1. **Build** via WSL2 Ubuntu (Rust cross-compile for Linux x86_64)
2. **Copy** binaries from WSL filesystem to Windows staging (`/c/Users/sync/`)
3. **SCP** to VBox VM at port 2222
4. **Kill** old yantrik-ui process, copy to `/opt/yantrik/bin/`, restart as `yantrik` user
5. **Verify** process is running

```bash
# Full build + deploy
bash deploy.sh

# Deploy only (skip build, use existing binaries)
bash deploy.sh --skip-build
```

**Note**: `deploy.sh` currently points to the OneDrive repo path for builds. If building from this clean clone, update the `cd` path in the build step to `/mnt/c/Users/sync/codes/yantrik-os`.

---

## CLI Feedback Loop — Testing Yantrik

### 1. Tail logs (primary feedback)

```bash
ssh -p 2222 -i /c/Users/sync/.ssh/id_deploy root@127.0.0.1 \
  "tail -f /opt/yantrik/logs/yantrik-os.log"
```

Shows: LLM tool calls, SystemObserver events, ProactiveFeature urges, WhisperCard lifecycle, errors.

### 2. Send D-Bus notifications (test NotificationRelay)

```bash
# Single notification
ssh -p 2222 -i /c/Users/sync/.ssh/id_deploy root@127.0.0.1 "su - yantrik -c '
  DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
  notify-send \"TestApp\" \"Hello from CLI\" -u normal
'"

# Test grouping (3+ from same app within 60s)
for i in 1 2 3; do
  ssh -p 2222 -i /c/Users/sync/.ssh/id_deploy root@127.0.0.1 "su - yantrik -c '
    DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
    notify-send \"Telegram\" \"Message $i\" -u normal
  '"
done
```

### 3. Take screenshots (visual feedback)

```bash
# From Windows host
VBoxManage.exe controlvm "Yantrik-OS" screenshotpng screenshot.png
```

### 4. Check process/resource state

```bash
ssh -p 2222 -i /c/Users/sync/.ssh/id_deploy root@127.0.0.1 "ps aux | grep yantrik"
ssh -p 2222 -i /c/Users/sync/.ssh/id_deploy root@127.0.0.1 "free -h"
ssh -p 2222 -i /c/Users/sync/.ssh/id_deploy root@127.0.0.1 "df -h /opt/yantrik"
```

### 5. LLM connection

- **Ollama** on Windows host `192.168.4.35:11434` (2x RTX 3090 Ti, CUDA)
- Model: **Qwen3.5-9B Q4_K_M**, configured in `/opt/yantrik/config.yaml`
- VM connects to host over VBox NAT network
- Test: `curl http://192.168.4.35:11434/api/tags`
