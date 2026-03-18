#!/bin/bash
# Test refined prompts on nemotron-3-nano:4b via Yantrik CLI in WSL
# Usage: bash scripts/test_prompts.sh

set -euo pipefail

run_test() {
    local name="$1"
    local query="$2"
    echo "=== TEST: $name ==="
    echo "Query: $query"
    echo "---"
    wsl.exe -d Ubuntu -- bash -lc \
        "/opt/yantrik/bin/yantrik ask --config /opt/yantrik/config.yaml --json '$query'" \
        2>/dev/null | python3 -c "
import sys, json
raw = sys.stdin.read().strip()
if not raw:
    print('(no output)')
    sys.exit()
try:
    data = json.loads(raw)
    resp = data.get('response', data.get('text', str(data)))
    print('Response:', resp[:400])
    tools = data.get('tool_calls', data.get('tools_used', []))
    if tools:
        if isinstance(tools, list):
            print('Tools:', [t.get('name', t) if isinstance(t, dict) else t for t in tools])
        else:
            print('Tools:', tools)
except Exception as e:
    print('Raw:', raw[:400])
" || echo "(command failed)"
    echo ""
    echo ""
}

echo "╔══════════════════════════════════════════╗"
echo "║  Yantrik Prompt Refinement Test Suite    ║"
echo "╚══════════════════════════════════════════╝"
echo ""

# 1. Direct answer (no tools needed)
run_test "Direct answer" "What is the capital of France?"

# 2. Tool selection — should pick recall
run_test "Memory recall" "What is my name?"

# 3. Tool selection — should pick web_search or get_weather
run_test "Weather" "What is the weather like today?"

# 4. Anti-fabrication — should NOT invent a price
run_test "Anti-fabrication" "How much does a Tesla Model 3 cost?"

# 5. Conciseness test
run_test "Concise response" "Tell me a joke"

# 6. Tool selection — should pick current_time
run_test "Current time" "What time is it?"

# 7. Clarification — should ask, not guess
run_test "Ask before guessing" "Send an email to John"

# 8. Memory save — should call remember
run_test "Remember fact" "I live in Austin, Texas"

# 9. System command
run_test "System info" "How much disk space do I have?"

# 10. Direct knowledge
run_test "Direct knowledge" "Explain recursion in one sentence"

echo "=== ALL TESTS COMPLETE ==="
