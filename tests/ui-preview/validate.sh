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
run verify-monitor
run verify-weather
run target/ui-validation/weather.png 1000 720 verify-weather-snapped
run target/ui-validation/installer.png 1280 800 verify-installer
run target/ui-validation/editor.png 1100 760 verify-editor
run target/ui-validation/unused.png 800 600 verify-launcher
run target/ui-validation/decision.png 720 780 verify-decision
run target/ui-validation/unused.png 1280 800 verify-apps
run target/ui-validation/mind-panel.png 1280 800 verify-mind-panel
run target/ui-validation/minds.png 1280 800 verify-minds
run target/ui-validation/jump-to-present.png 480 400 verify-jump-to-present
run target/ui-validation/context-rail.png 900 500 verify-context-rail
run target/ui-validation/calendar.png 1100 720 verify-calendar
run target/ui-validation/provider-panel.png 1280 800 verify-provider-panel
run target/ui-validation/handoff-card.png 1280 800 verify-handoff-card
run target/ui-validation/accounts-page.png 1280 800 verify-accounts-page
run target/ui-validation/ai-map.png 1280 800 verify-ai-map
run target/ui-validation/free-ai.png 600 1800 verify-free-ai
run target/ui-validation/providers-in-use.png 1280 2400 verify-providers-in-use
run target/ui-validation/unused.png 1280 800 verify-screen-controls
run target/ui-validation/apps-button.png 1280 800 verify-apps-button
run target/ui-validation/launcher.png 1280 800 verify-launcher-scenes
run target/ui-validation/taskbar-menu.png 1280 800 verify-taskbar-menu
run target/ui-validation/dock.png 1280 800 verify-dock
run target/ui-validation/alt-tab.png 1280 800 verify-alt-tab
run target/ui-validation/alt-tab-narrow.png 700 700 verify-alt-tab
run target/ui-validation/cards-waiting.png 1280 800 verify-cards-waiting
run target/ui-validation/kit-controls.png 880 460 verify-kit-controls
run target/ui-validation/colour-system.png 720 360 verify-colour-system
run target/ui-validation/icons.png 720 400 verify-icons
run target/ui-validation/bar-overlays.png 1280 800 verify-bar-overlays
run target/ui-validation/cheat-sheet.png 1280 800 verify-cheat-sheet
run target/ui-validation/quick-settings.png 1280 800 verify-quick-settings
run target/ui-validation/osd.png 1280 800 verify-osd
run target/ui-validation/battery.png 1280 800 verify-battery
run target/ui-validation/network.png 1280 800 verify-network
run target/ui-validation/lens-answers.png 640 800 verify-lens-answers
run target/ui-validation/approval-card.png 1280 800 verify-approval-card
run target/ui-validation/chat.png 1280 800 verify-chat
run target/ui-validation/approval-pointer-only.png 1280 800 verify-approval-pointer-only
for scene in notes files settings desktop agent; do
    run "target/ui-validation/$scene.png" 1280 800 "$scene"
    run "target/ui-validation/$scene-compact.png" 800 600 "$scene"
done
run target/ui-validation/notes-light.png 1100 720 notes light
