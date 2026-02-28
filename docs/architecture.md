# Yantrik OS — System Architecture

> Decision document. Finalized via multi-model brainstorm (Claude + GPT-4.1 + DeepSeek).
> Last updated: 2026-02-27

---

## Overview

Yantrik OS is an AI-native desktop OS where the AI IS the shell. Single Rust binary:
Slint UI + CompanionService + Candle LLM + YantrikDB. Zero external LLM dependencies.

Stack: Alpine Linux → labwc (Wayland compositor) → yantrik-ui (fullscreen Slint shell)

---

## 3 Crates

```
crates/
  yantrik-os/              # System integration (D-Bus, inotify, sysinfo). ZERO AI deps.
  yantrikdb-companion/     # AI brain (already exists). LLM, memory, instincts, bond.
  yantrik-ui/              # Shell binary. Features, UI, orchestration.
```

Dependency graph: `yantrik-os → yantrik-ui ← yantrikdb-companion` (shell orchestrates both)

### yantrik-os
Pure system observation. No AI, no Slint, no LLM. Watches the machine via D-Bus,
inotify, and sysinfo. Emits `SystemEvent` variants over a crossbeam channel.

### yantrikdb-companion
The AI brain. Already fully built: 9 instincts, bond system, Candle LLM (Qwen2.5-0.5B),
MiniLM embeddings, YantrikDB memory engine, evolution/learning pipelines.

### yantrik-ui
The shell binary that ties everything together. Owns the Slint event loop, runs
ProactiveFeatures, routes system events to the brain, renders urges as Whisper Cards.

---

## 3 Threads

```
Thread 1: MAIN — Slint event loop
  - Owns all UI state (Slint properties = source of truth)
  - Polls crossbeam receivers via Slint Timer (every 2-5s)
  - Runs FeatureRegistry.process_event() and .tick()
  - Updates UI with scored urges

Thread 2: SYSTEM OBSERVER (yantrik-os)
  - D-Bus listeners (UPower, NetworkManager, Notifications)
  - inotify file watchers (~/, ~/Downloads, ~/Documents)
  - sysinfo process polling (every 5s)
  - Sends SystemEvent via crossbeam channel → main thread

Thread 3: COMPANION WORKER (existing bridge.rs)
  - LLM inference (Candle, synchronous)
  - Memory DB queries/writes (YantrikDB)
  - Instinct evaluation, bond computation
  - Sends CompanionResponse via crossbeam channel → main thread
```

### Why 3 Threads?
- Slint requires main thread for UI rendering
- LLM inference blocks for seconds — can't be on the main thread
- D-Bus/inotify listeners need their own event loops
- crossbeam channels are the simplest correct solution

---

## 3 Architectural Invariants

1. **Features are pure**: `(events, state) → urges`. No side effects, no direct UI mutation.
2. **FeatureContext is read-only**: Features can query memory DB but can't write.
3. **All inter-thread communication via crossbeam channels**. No tokio, no async, no Arc<Mutex>.

---

## ProactiveFeature Trait

```rust
pub trait ProactiveFeature {
    fn name(&self) -> &str;
    fn on_event(&mut self, event: &SystemEvent, ctx: &FeatureContext) -> Vec<Urge>;
    fn on_tick(&mut self, ctx: &FeatureContext) -> Vec<Urge>;
    fn on_feedback(&mut self, urge_id: &str, outcome: Outcome, ctx: &FeatureContext);
}

pub struct FeatureContext<'a> {
    pub system: &'a SystemSnapshot,     // Current system state
    pub memory_query: &'a dyn MemoryQuery,  // Read-only memory access
    pub companion: &'a CompanionSnapshot,   // Bond level, mood, etc.
    pub config: &'a FeatureConfig,      // Per-feature YAML config
    pub clock: DateTime<Local>,         // Current time
}
```

**Adding a new feature** = one file + implement trait + one `registry.register()` call.
No crate changes needed. No wiring. No new channels.

---

## Event Flow

```
SystemObserver (Thread 2)
  → crossbeam channel
    → Main thread Timer callback (every 2-5s)
      → system_state.apply(event)           // Update SystemSnapshot
      → registry.process_event(event, ctx)  // All features evaluate
        → Vec<Urge>
      → urgency_scorer.filter(urges)        // pressure = urgency × confidence × interruptibility
        → Vec<ScoredUrge>
      → Update Slint UI (Whisper Cards, status bar)
      → Record event in memory DB via companion bridge
```

### Urgency Scoring

```
pressure = urgency × confidence × interruptibility

Tiers:
  pressure ≥ 0.85  →  Interrupt (floating card, sound)
  pressure ≥ 0.50  →  Whisper (subtle card, no sound)
  pressure ≥ 0.25  →  Queue (visible in Quiet Queue only)
  pressure < 0.25  →  Drop
```

---

## Configuration

Single YAML file: `config/yantrik-os.yaml`

Each feature declares its config schema via serde. Example:

```yaml
features:
  resource_guardian:
    enabled: true
    battery_warning_threshold: 20
    disk_warning_threshold_percent: 10
    cpu_sustained_threshold: 90
    cpu_sustained_seconds: 60
  process_sentinel:
    enabled: true
    poll_interval_secs: 5
  focus_flow:
    enabled: true
    deep_work_threshold_mins: 20
    break_reminder_mins: 90
```

---

## Security Model

### v1: Safety by Types
Features can't do dangerous things because `FeatureContext` is read-only.
They can only output `Urge` values — the orchestration layer decides what to do.

### v2+: Permission System
Features output `UrgeAction::RequestAction` for dangerous operations.
Orchestration layer checks user-configured permissions before executing.

```rust
pub enum PermissionLevel {
    AlwaysAllow,    // Read battery, list windows
    AskOnce,        // Open app, read clipboard
    AskEvery,       // Type into app, close app, delete file
    NeverAllow,     // Send email, make purchase
}
```

---

## File Structure

```
crates/yantrik-os/src/
  lib.rs              # Re-exports
  events.rs           # SystemEvent enum
  observer.rs         # SystemObserver (spawns listeners, fans into one channel)
  battery.rs          # UPower D-Bus listener
  network.rs          # NetworkManager D-Bus listener
  notifications.rs    # org.freedesktop.Notifications listener
  files.rs            # inotify file watcher
  processes.rs        # sysinfo process tracker (poll every 5s)
  mock.rs             # Mock mode for QEMU dev (fake battery drain, etc.)

crates/yantrik-ui/src/
  main.rs             # Thin: init → event loop → shutdown
  bridge.rs           # CompanionService bridge (existing)
  ui_state.rs         # Centralized Slint property updates
  features/
    mod.rs            # ProactiveFeature trait, FeatureRegistry, UrgencyScorer
    resource_guardian.rs
    process_sentinel.rs
    error_companion.rs
    file_lifecycle.rs
    focus_flow.rs
```

---

## Testing Strategy

Features are tested with mock `FeatureContext`:

```rust
#[test]
fn battery_below_threshold_fires_urge() {
    let mut guardian = ResourceGuardian::new();
    let ctx = FeatureContext {
        system: &SystemSnapshot { battery_level: 15, .. },
        memory_query: &MockMemory::empty(),
        ..test_context()
    };
    let event = SystemEvent::BatteryChanged { level: 15, charging: false, .. };
    let urges = guardian.on_event(&event, &ctx);
    assert_eq!(urges.len(), 1);
    assert!(urges[0].urgency > 0.8);
}
```

No Slint, no system access, no D-Bus. Pure logic tests.

---

## v1 Proactive Features

### 1. Resource Guardian
- **Monitors**: Battery (UPower/D-Bus), CPU/RAM (sysinfo), disk (statfs), thermal
- **Thresholds**: Battery <20% → high, disk <10% → high, CPU >90% sustained 60s → medium
- **LLM**: Template + context vars → natural phrasing
- **Bonus**: Celebrate recovery ("You freed 3GB — nice!")

### 2. Process Sentinel
- **Monitors**: Process list diffs (sysinfo, 5s poll)
- **Flags**: First-seen process with high CPU/network → alert
- **Trust flywheel**: User says "that's fine" → never flag again
- **No LLM needed** — pure heuristic

### 3. Error Companion (Killer Feature)
- **Monitors**: Process exit codes ≠ 0, signals (SIGSEGV, SIGABRT), terminal errors
- **Terminal capture**: PROMPT_COMMAND hook → logs command + exit code → inotify
- **On error**: Embed text → semantic search memory DB → surface past fix
- **Learning**: "I'll remember your solution next time" → personal troubleshooting KB

### 4. File Lifecycle Tracker
- **Monitors**: ~/Downloads, ~/Documents, ~/Desktop via inotify
- **Triggers**: Downloaded not opened (2hr), edited then abandoned (7d),
  large file >500MB (3d), burst downloads (>10 files/hr)
- **Memory DB**: Records all file events with timestamps

### 5. Focus Flow
- **Monitors**: App switch frequency, idle detection
- **States**: DEEP_WORK (no switch >20min), SCATTERED (>5 switches/min for 3min), NORMAL
- **DEEP_WORK mode**: Suppress whispers below urgency 0.85
- **Break reminder**: After 90min continuous, surface during next idle window

### Implementation Order
1. SystemObserver crate (yantrik-os) — D-Bus + inotify + sysinfo
2. HeuristicEngine — threshold checks, process diffs, file lifecycle rules
3. Whisper Card UI component — floating urgency-driven notification cards
4. Wire into existing urge_queue → UI pipeline
5. Error Companion shell integration (PROMPT_COMMAND hook)
6. Focus Flow state machine

### NOT v1 (Needs Infrastructure)
- RSS/feed filtering
- Calendar integration
- Email/messaging bridge
- Finance/bank statement parsing
- Browser tab monitoring

---

## User Feedback Loop

Every Whisper Card outcome is tracked:
1. **User acts on it** → high signal (useful)
2. **User dismisses** → low signal (bad timing or irrelevant)
3. **Decays naturally** → urgency was too low

After 30 days, urgency thresholds personalize per feature per user.
LLM called ~5-10x/day total — everything else is heuristic.
