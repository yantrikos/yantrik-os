#!/bin/sh
# Debug display/rendering inside Yantrik OS VM

echo "=== DRM modes on card0 (virtio-gpu) ==="
cat /sys/class/drm/card0-*/modes 2>/dev/null | head -5 || echo "no modes"

echo
echo "=== DRM mode on card1 (bochs) ==="
cat /sys/class/drm/card1-*/modes 2>/dev/null | head -5 || echo "no modes"

echo
echo "=== Framebuffer ==="
cat /sys/class/graphics/fb0/name 2>/dev/null
cat /sys/class/graphics/fb0/virtual_size 2>/dev/null

echo
echo "=== labwc PID ==="
LABWC_PID=$(ps aux | grep '[l]abwc' | awk '{print $1}')
echo "labwc PID: $LABWC_PID"

echo
echo "=== yantrik-ui PID ==="
UI_PID=$(ps aux | grep '[y]antrik-ui' | awk '{print $1}')
echo "UI PID: $UI_PID"

echo
echo "=== Wayland sockets ==="
ls -la /run/user/1000/

echo
echo "=== wlr-randr ==="
su -l yantrik -c 'XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 wlr-randr' 2>&1

echo
echo "=== yantrik-ui log (stderr from main thread) ==="
# Check if main thread is producing any output
cat /opt/yantrik/logs/yantrik-os.log 2>/dev/null | grep -v "EGL\|egl\|xwayland\|Xwayland\|libEGL" | tail -10

echo
echo "=== kernel DRM messages ==="
dmesg | grep -i "drm\|virtio.*gpu\|fb\|display" | tail -10
