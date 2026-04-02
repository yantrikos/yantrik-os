# Contributing to Yantrik OS

## Architecture

The runtime is still organized around **three cooperating roles** (same mental model as the README):

1. **Main thread** — Slint UI (event loop, rendering)
2. **SystemObserver** — D-Bus, inotify, sysinfo polling (battery, WiFi, processes, idle)
3. **Companion worker** — LLM inference, memory search, tool execution

The **Cargo workspace** is larger than the classic “five crates” diagram. The authoritative list of packages is **`[workspace].members` in the repo root `Cargo.toml`**.

### Layout (summary)

**Crates (`crates/`):**

| Crate | Role |
|-------|------|
| `yantrik` | Main binary / composition |
| `yantrik-ui` | Desktop shell, Slint, `wire/` callbacks |
| `yantrik-os` | System observer, platform integration |
| `yantrik-companion` | Agent loop, wiring to tools and ML |
| `yantrik-companion-core` | `Tool` trait, `ToolContext`, shared types |
| `yantrik-companion-tools` | **Tool implementations** (one module per domain) |
| `yantrik-companion-instincts` | Proactive instincts |
| `yantrik-companion-cortex` | Cortex / pattern logic |
| `yantrik-chat` | Chat-side plumbing |
| `yantrik-ml` | LLM backends (also pulled in via workspace dependencies / path patches) |
| `yantrikdb-core`, `yantrikdb-server` | Memory DB and server |
| `yantrik-app-runtime` | App host / runtime |
| `yantrik-ui-slint`, `yantrik-ui-kit`, `yantrik-shell-core` | UI layers and shell primitives |
| `yantrik-ipc-contracts`, `yantrik-ipc-transport` | IPC |
| `yantrik-service-sdk`, `yantrik-manifest`, `yantrik-design-tokens` | Shared infrastructure |

**Apps (`apps/`)** — One package per built-in app (e.g. `spreadsheet`, `email`, `terminal`).

**Services (`services/`)** — Background services (e.g. `email-service`, `calendar-service`).

### Wire Pattern

Each UI feature lives in `crates/yantrik-ui/src/wire/<name>.rs` with:

```rust
pub fn wire(ui: &App, ctx: &AppContext) {
    // Register Slint callbacks here
}
```

Registered once in [`crates/yantrik-ui/src/wire/mod.rs`](../crates/yantrik-ui/src/wire/mod.rs). This keeps entrypoints thin when adding features.

### Tool Pattern

Each tool category lives in **`crates/yantrik-companion-tools/src/<name>.rs`**. The crate re-exports the `Tool` trait from `yantrik-companion-core` (see [`lib.rs`](../crates/yantrik-companion-tools/src/lib.rs)).

```rust
pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(MyTool));
}

struct MyTool;

impl Tool for MyTool {
    fn name(&self) -> &'static str { "my_tool" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "my_category" }
    fn definition(&self) -> serde_json::Value { /* OpenAI function schema */ }
    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String { /* ... */ }
}
```

Permission levels: `Safe` (read-only) < `Standard` (reversible writes) < `Sensitive` (system changes) < `Dangerous` (destructive).

---

## Adding a New Tool

1. Create `crates/yantrik-companion-tools/src/mytool.rs`
2. Implement `Tool` for each tool (see [`git.rs`](../crates/yantrik-companion-tools/src/git.rs) for a full example with `register()`)
3. Add `pub mod mytool;` to [`lib.rs`](../crates/yantrik-companion-tools/src/lib.rs) and call `mytool::register(reg)` from `register_all()`
4. Run `cargo check -p yantrik-companion-tools` (and `cargo check -p yantrik-companion` if you change how tools are wired in the main agent)

Some tools need config from the running app (e.g. canvas, vision, github) and are **registered from `yantrik-companion`** instead of inside `register_all()`—follow existing patterns in `lib.rs` comments when adding similar tools.

**Tips:**
- Shell out to system CLIs when possible (no extra Rust deps)
- Truncate output to ~3000 chars for LLM-friendly responses
- Use `validate_path()` for file access (blocks `.ssh`, `.gnupg`, etc.)
- Use `expand_home()` for `~/` path expansion
- Store audit memories via `ctx.db.record_text()` for important operations

---

## Creating a YAML Plugin

Plugins add tools without writing Rust. Place `.yaml` files in `~/.config/yantrik/plugins/`:

```yaml
name: "my-tools"
version: "1.0"
tools:
  - name: "check_vpn"
    description: "Check if VPN is connected"
    permission: "safe"
    category: "network"
    parameters: {}
    command: "mullvad status"

  - name: "deploy_staging"
    description: "Deploy branch to staging"
    permission: "sensitive"
    category: "devops"
    parameters:
      branch:
        type: "string"
        description: "Branch name"
        required: true
    command: "cd ~/projects && ./deploy.sh {branch}"
```

**Rules:**
- Parameters substitute into the command as `{param_name}`
- Parameter values are sanitized (no `;`, `&`, `|`, `` ` ``, `$`, `>`, `<`)
- Command templates are trusted (written by plugin author)
- Permission ceiling from config's `max_permission` still applies

---

## Creating a Community Theme

Place a `theme-override.yaml` in `~/.config/yantrik/`:

```yaml
name: "Nord"
enabled: true
bg_deep: "#2e3440"
bg_surface: "#3b4252"
bg_card: "#434c5e"
bg_elevated: "#4c566a"
amber: "#ebcb8b"
cyan: "#88c0d0"
text_primary: "#eceff4"
text_secondary: "#d8dee9"
text_dim: "#4c566a"
accent: "#81a1c1"
```

**Available tokens:** `bg_deep`, `bg_surface`, `bg_card`, `bg_elevated`, `amber`, `cyan`, `text_primary`, `text_secondary`, `text_dim`, `accent`.

Set `enabled: false` or delete the file to revert to the default Firelight theme.

---

## Dev Environment Setup

### Prerequisites

- Windows 11 with WSL2 (Ubuntu 24.04)
- Rust 1.93+ in WSL2
- QEMU with KVM acceleration

### Build

```bash
# From WSL2
cd /mnt/c/Users/<you>/path/to/yantrik-os
CARGO_TARGET_DIR=/home/<user>/target-yantrik cargo check

# Check specific workspace members (non-exhaustive)
cargo check -p yantrik-ui
cargo check -p yantrik-companion
cargo check -p yantrik-companion-tools
cargo check -p yantrik-os
cargo check -p yantrik
```

### Run in QEMU

```bash
# Deploy Alpine VM
bash deploy/yantrik-os/setup-alpine-vm.sh

# Boot desktop
bash deploy/yantrik-os/boot-desktop.sh
```

### Known Issues

- **rustc 1.93.x internal compiler error (ICE) around dead-code / early lints**  
  This is a **compiler bug**, not bad code in Yantrik. The `yantrik-ui` crate already carries mitigations: see **`crates/yantrik-ui/src/main.rs`** (`#![allow(unused)]` at crate root and `#[allow(dead_code)]` on the `mod` lines called out in comments there).  
  If you still hit an ICE on **`cargo check`** with another crate, use the stack trace to find the **`mod`** involved and add `#[allow(dead_code)]` on that declaration, or try **`rustup update`** to a newer stable toolchain where the bug may already be fixed.

- **`[patch]` in the root `Cargo.toml`**  
  The workspace may patch `yantrik-ml` / `yantrikdb-core` to **path dependencies** for in-tree builds. That is intentional for this repo. Only edit `[patch]` when you are deliberately developing those libraries from separate local checkouts alongside `yantrik-os`; otherwise leave it as committed.
