# Contributing to Yantrik OS

## Architecture

Yantrik OS is a single Rust binary that combines three threads:

1. **Main thread** — Slint UI (event loop, rendering)
2. **SystemObserver thread** — D-Bus, inotify, sysinfo polling (battery, WiFi, processes, idle)
3. **Companion Worker thread** — LLM inference, memory search, tool execution

```
crates/
  yantrik-os/        # SystemObserver, events, platform abstraction
  yantrikdb-core/    # Memory DB (SQLite + HNSW vector search, knowledge graph, vault)
  yantrik-ml/        # LLM backends (Ollama API, OpenAI API, Claude CLI, llama.cpp)
  yantrik-companion/ # Agent brain: tools, instincts, bond, personality, cortex
  yantrik-ui/        # Slint desktop shell (all UI, wiring, features)
```

### Wire Pattern

Each UI feature lives in `crates/yantrik-ui/src/wire/<name>.rs` with:

```rust
pub fn wire(ui: &App, ctx: &AppContext) {
    // Register Slint callbacks here
}
```

Registered once in `wire/mod.rs`. This keeps `main.rs` untouched when adding features.

### Tool Pattern

Each tool category lives in `crates/yantrikdb-companion/src/tools/<name>.rs`:

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

1. Create `crates/yantrik-companion/src/tools/mytool.rs`
2. Implement `Tool` trait for each tool (see `git.rs` for a complete example)
3. Add `pub mod mytool;` to `tools/mod.rs`
4. Add `mytool::register(&mut reg);` in `build_registry()`
5. Run `cargo check -p yantrik-companion`

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

# Check specific crate
cargo check -p yantrik-ui
cargo check -p yantrik-companion
cargo check -p yantrik-ml
```

### Run in QEMU

```bash
# Deploy Alpine VM
bash deploy/yantrik-os/setup-alpine-vm.sh

# Boot desktop
bash deploy/yantrik-os/boot-desktop.sh
```

### Known Issues

- **rustc 1.93.1 ICE in `check_mod_deathness`**: Add `#[allow(dead_code)]` to affected module declarations
- **Private repos**: yantrik-ml, yantrikdb-core, yantrik-companion are private. Needs `gh auth token` for cargo git deps. Use workspace `[patch]` sections for local development.
