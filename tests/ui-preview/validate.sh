#!/usr/bin/env bash
# Run from any directory; artifacts stay under the repository's ignored target/.
set -euo pipefail
cd "$(dirname "$0")/../.."
mkdir -p target/ui-validation
manifest=tests/ui-preview/Cargo.toml
cargo build --offline --manifest-path "$manifest" --profile fast
run() { cargo run --offline --quiet --manifest-path "$manifest" --profile fast -- "$@"; }
run verify-overflow
run verify-idle
run verify-controls
run target/ui-validation/unused.png 800 600 verify-launcher
run target/ui-validation/unused.png 1280 800 verify-apps
run target/ui-validation/mind-panel.png 1280 800 verify-mind-panel
run target/ui-validation/lens-answers.png 640 800 verify-lens-answers
run target/ui-validation/agents-overview.png 1280 800 verify-agents-overview
for scene in notes files settings desktop agent; do
    run "target/ui-validation/$scene.png" 1280 800 "$scene"
    run "target/ui-validation/$scene-compact.png" 800 600 "$scene"
done
run target/ui-validation/notes-light.png 1100 720 notes light
