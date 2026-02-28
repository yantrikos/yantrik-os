#!/bin/sh
# Start Yantrik OS desktop via labwc compositor
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
mkdir -p "$XDG_RUNTIME_DIR"

# Log file
LOG=/opt/yantrik/logs/yantrik-os.log

echo "$(date): Starting Yantrik OS desktop..." >> "$LOG"

# labwc reads its config from ~/.config/labwc/
# Environment vars (LD_PRELOAD, WLR_*, SLINT_BACKEND) are in ~/.config/labwc/environment
# yantrik-ui auto-starts via ~/.config/labwc/autostart
exec labwc >> "$LOG" 2>&1
