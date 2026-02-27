#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Yantrik OS — Deploy Full Stack
# ═══════════════════════════════════════════════════════════════
#
# Run this ON the Yantrik OS device (VM, phone, or any Alpine/postmarketOS).
# Installs: YantrikDB engine, Python companion service, llama.cpp, embeddings.
#
# Usage:
#   chmod +x deploy-stack.sh
#   sudo ./deploy-stack.sh
#
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

YANTRIK_HOME="/opt/yantrik"
YANTRIK_USER="yantrik"
DATA_DIR="$YANTRIK_HOME/data"
LOG_DIR="$YANTRIK_HOME/logs"

echo "═══════════════════════════════════════════════"
echo "  Yantrik Stack Deployment"
echo "═══════════════════════════════════════════════"
echo

# ── Detect OS ──
if [ -f /etc/alpine-release ]; then
    PKG_MGR="apk"
    echo "Detected: Alpine Linux $(cat /etc/alpine-release)"
elif [ -f /etc/debian_version ]; then
    PKG_MGR="apt"
    echo "Detected: Debian/Ubuntu"
else
    PKG_MGR="unknown"
    echo "Warning: Unknown OS, attempting Alpine commands"
    PKG_MGR="apk"
fi

# ── Step 1: System packages ──
echo
echo "[1/7] Installing system packages..."
if [ "$PKG_MGR" = "apk" ]; then
    apk update -q
    apk add -q \
        python3 py3-pip py3-virtualenv py3-sqlite3 \
        sqlite curl wget git \
        build-base python3-dev \
        openrc dbus
elif [ "$PKG_MGR" = "apt" ]; then
    apt-get update -qq
    apt-get install -y -qq \
        python3 python3-pip python3-venv python3-dev \
        sqlite3 curl wget git build-essential
fi

# ── Step 2: Create yantrik user and directories ──
echo "[2/7] Setting up Yantrik directories..."
id "$YANTRIK_USER" &>/dev/null || adduser -D -h "$YANTRIK_HOME" "$YANTRIK_USER" 2>/dev/null || true
mkdir -p "$YANTRIK_HOME" "$DATA_DIR" "$LOG_DIR" "$YANTRIK_HOME/wheels"
chown -R "$YANTRIK_USER:$YANTRIK_USER" "$YANTRIK_HOME" 2>/dev/null || true

# ── Step 3: Install llama.cpp ──
echo "[3/7] Installing llama.cpp..."
if ! command -v llama-server &>/dev/null; then
    LLAMA_DIR="$YANTRIK_HOME/llama.cpp"
    if [ ! -d "$LLAMA_DIR" ]; then
        git clone --depth 1 https://github.com/ggerganov/llama.cpp.git "$LLAMA_DIR"
    fi
    cd "$LLAMA_DIR"
    make -j$(nproc) llama-server 2>&1 | tail -3
    cp llama-server /usr/local/bin/
    echo "  llama-server installed"
else
    echo "  llama-server already installed"
fi

# ── Step 4: Download models ──
echo "[4/7] Downloading LLM models..."
MODEL_DIR="$YANTRIK_HOME/models"
mkdir -p "$MODEL_DIR"

# Chat model: Qwen2.5-1.5B (small, fast, good quality)
CHAT_MODEL="$MODEL_DIR/qwen2.5-1.5b-instruct-q4_k_m.gguf"
if [ ! -f "$CHAT_MODEL" ]; then
    echo "  Downloading Qwen2.5-1.5B chat model (~1GB)..."
    wget -q --show-progress -O "$CHAT_MODEL" \
        "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf" || {
        echo "  Warning: Download failed. You can manually download the model later."
    }
else
    echo "  Chat model already present"
fi

# Embedding model: all-MiniLM-L6-v2 (384-dim, tiny)
EMBED_MODEL="$MODEL_DIR/all-minilm-l6-v2-q8_0.gguf"
if [ ! -f "$EMBED_MODEL" ]; then
    echo "  Downloading MiniLM embedding model (~23MB)..."
    wget -q --show-progress -O "$EMBED_MODEL" \
        "https://huggingface.co/leliuga/all-MiniLM-L6-v2-GGUF/resolve/main/all-MiniLM-L6-v2.Q8_0.gguf" || {
        echo "  Warning: Download failed. You can manually download the model later."
    }
else
    echo "  Embedding model already present"
fi

# ── Step 5: Python virtual environment + YantrikDB ──
echo "[5/7] Setting up Python environment..."
VENV="$YANTRIK_HOME/venv"
if [ ! -d "$VENV" ]; then
    python3 -m venv "$VENV"
fi
source "$VENV/bin/activate"

pip install --quiet --upgrade pip

# Install YantrikDB wheel if present
WHEEL=$(ls "$YANTRIK_HOME/wheels/"yantrikdb*.whl 2>/dev/null | head -1)
if [ -n "$WHEEL" ]; then
    echo "  Installing YantrikDB from wheel: $(basename $WHEEL)"
    pip install --quiet "$WHEEL"
else
    echo "  Warning: No YantrikDB wheel found in $YANTRIK_HOME/wheels/"
    echo "  Build it on your dev machine and copy here:"
    echo "    cd /path/to/yantrikdb && maturin build --release"
    echo "    scp target/wheels/yantrikdb-*.whl yantrik@device:$YANTRIK_HOME/wheels/"
fi

# Install companion dependencies
pip install --quiet httpx pyyaml fastapi uvicorn

# ── Step 6: Configuration ──
echo "[6/7] Writing configuration..."
cat > "$YANTRIK_HOME/config.yaml" <<'CONFIG'
# Yantrik Companion Configuration
user_name: "Pranab"

llm:
  base_url: "http://127.0.0.1:8081/v1"
  model: "qwen2.5"
  max_tokens: 512
  temperature: 0.7
  max_context_tokens: 2048

embedding:
  base_url: "http://127.0.0.1:8082/v1"
  model: "all-minilm"
  dim: 384

conversation:
  max_history_turns: 10
  session_timeout_minutes: 30

tools:
  enabled: true
  max_tool_rounds: 3

urges:
  proactive_urgency_threshold: 0.7
  max_pending: 20
  expiry_hours: 24

instincts:
  enabled:
    - check_in
    - emotional_awareness
    - follow_up
    - pattern_surfacing
    - conflict_alerting
  check_in:
    idle_hours_threshold: 8
  emotional_awareness:
    negative_valence_threshold: -0.3
  follow_up:
    aging_hours: 4

background:
  think_interval_active: 900    # 15 min
  think_interval_idle: 1800     # 30 min
  urge_expiry_check: 3600       # 1 hour

db:
  path: "/opt/yantrik/data/memory.db"
  embedding_dim: 384
CONFIG

# ── Step 7: Systemd/OpenRC services ──
echo "[7/7] Setting up services..."

# Detect init system
if command -v systemctl &>/dev/null; then
    INIT="systemd"
elif command -v rc-service &>/dev/null; then
    INIT="openrc"
else
    INIT="none"
fi

if [ "$INIT" = "openrc" ]; then
    # llama-server (chat)
    cat > /etc/init.d/llama-server <<'SVC'
#!/sbin/openrc-run
name="llama-server"
description="llama.cpp chat inference server"
command="/usr/local/bin/llama-server"
command_args="-m /opt/yantrik/models/qwen2.5-1.5b-instruct-q4_k_m.gguf --host 127.0.0.1 --port 8081 -ngl 0 -c 2048"
command_user="yantrik"
command_background="yes"
pidfile="/run/${RC_SVCNAME}.pid"
output_log="/opt/yantrik/logs/llama-server.log"
error_log="/opt/yantrik/logs/llama-server.log"
SVC
    chmod +x /etc/init.d/llama-server

    # llama-server (embeddings)
    cat > /etc/init.d/llama-embed <<'SVC'
#!/sbin/openrc-run
name="llama-embed"
description="llama.cpp embedding server"
command="/usr/local/bin/llama-server"
command_args="-m /opt/yantrik/models/all-minilm-l6-v2-q8_0.gguf --host 127.0.0.1 --port 8082 --embedding -ngl 0 -c 512"
command_user="yantrik"
command_background="yes"
pidfile="/run/${RC_SVCNAME}.pid"
output_log="/opt/yantrik/logs/llama-embed.log"
error_log="/opt/yantrik/logs/llama-embed.log"
SVC
    chmod +x /etc/init.d/llama-embed

    # yantrik-companion
    cat > /etc/init.d/yantrik-companion <<'SVC'
#!/sbin/openrc-run
name="yantrik-companion"
description="Yantrik AI Companion Service"
command="/opt/yantrik/venv/bin/python"
command_args="-m uvicorn yantrikdb.agent.service:app --host 0.0.0.0 --port 8340"
command_user="yantrik"
command_background="yes"
pidfile="/run/${RC_SVCNAME}.pid"
output_log="/opt/yantrik/logs/companion.log"
error_log="/opt/yantrik/logs/companion.log"
directory="/opt/yantrik"
depend() {
    need llama-server llama-embed
}
SVC
    chmod +x /etc/init.d/yantrik-companion

    # Enable services
    rc-update add llama-server default 2>/dev/null || true
    rc-update add llama-embed default 2>/dev/null || true
    rc-update add yantrik-companion default 2>/dev/null || true

    echo "  OpenRC services configured"

elif [ "$INIT" = "systemd" ]; then
    # llama-server (chat)
    cat > /etc/systemd/system/llama-server.service <<'SVC'
[Unit]
Description=llama.cpp chat inference server
After=network.target

[Service]
Type=simple
User=yantrik
ExecStart=/usr/local/bin/llama-server -m /opt/yantrik/models/qwen2.5-1.5b-instruct-q4_k_m.gguf --host 127.0.0.1 --port 8081 -ngl 0 -c 2048
Restart=on-failure
MemoryMax=2G

[Install]
WantedBy=multi-user.target
SVC

    # llama-server (embeddings)
    cat > /etc/systemd/system/llama-embed.service <<'SVC'
[Unit]
Description=llama.cpp embedding server
After=network.target

[Service]
Type=simple
User=yantrik
ExecStart=/usr/local/bin/llama-server -m /opt/yantrik/models/all-minilm-l6-v2-q8_0.gguf --host 127.0.0.1 --port 8082 --embedding -ngl 0 -c 512
Restart=on-failure
MemoryMax=256M

[Install]
WantedBy=multi-user.target
SVC

    # yantrik-companion
    cat > /etc/systemd/system/yantrik-companion.service <<'SVC'
[Unit]
Description=Yantrik AI Companion Service
After=llama-server.service llama-embed.service

[Service]
Type=simple
User=yantrik
WorkingDirectory=/opt/yantrik
ExecStart=/opt/yantrik/venv/bin/python -m uvicorn yantrikdb.agent.service:app --host 0.0.0.0 --port 8340
Restart=on-failure
MemoryMax=512M

[Install]
WantedBy=multi-user.target
SVC

    systemctl daemon-reload
    systemctl enable llama-server llama-embed yantrik-companion 2>/dev/null || true
    echo "  Systemd services configured"
else
    echo "  Warning: No init system detected. Services not configured."
    echo "  Start manually:"
    echo "    llama-server -m $CHAT_MODEL --host 127.0.0.1 --port 8081 -ngl 0 -c 2048 &"
    echo "    llama-server -m $EMBED_MODEL --host 127.0.0.1 --port 8082 --embedding -ngl 0 -c 512 &"
    echo "    $VENV/bin/python -m uvicorn yantrikdb.agent.service:app --host 0.0.0.0 --port 8340 &"
fi

chown -R "$YANTRIK_USER:$YANTRIK_USER" "$YANTRIK_HOME" 2>/dev/null || true

echo
echo "═══════════════════════════════════════════════"
echo "  Yantrik Stack Deployed!"
echo "═══════════════════════════════════════════════"
echo
echo "Services:"
echo "  llama-server     → 127.0.0.1:8081  (chat LLM)"
echo "  llama-embed      → 127.0.0.1:8082  (embeddings)"
echo "  yantrik-companion → 0.0.0.0:8340   (web UI + API)"
echo
echo "Start all:"
if [ "$INIT" = "openrc" ]; then
    echo "  rc-service llama-server start"
    echo "  rc-service llama-embed start"
    echo "  rc-service yantrik-companion start"
elif [ "$INIT" = "systemd" ]; then
    echo "  sudo systemctl start llama-server llama-embed yantrik-companion"
fi
echo
echo "Open in browser: http://localhost:8340"
echo
