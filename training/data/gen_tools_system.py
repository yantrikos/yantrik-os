#!/usr/bin/env python3
"""
Generate synthetic training data for SYSTEM tool family.
Output: batch_tools_01_system.jsonl (300 examples)
"""

import json
import random
import os
from pathlib import Path

random.seed(73)
OUT_DIR = Path(__file__).parent
OUT_FILE = OUT_DIR / "batch_tools_01_system.jsonl"

# ---------------------------------------------------------------------------
# Bond stage system prompts
# ---------------------------------------------------------------------------
BOND_PROMPTS = {
    "stranger": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: STRANGER. Be helpful, polite, slightly reserved. "
        "Do not assume familiarity. Use full sentences. No filler phrases, no emoji."
    ),
    "acquaintance": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: ACQUAINTANCE. Be friendly and warm. You know basic preferences. "
        "Concise, natural contractions. No filler phrases, no emoji."
    ),
    "trusted": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: TRUSTED. Casual and direct. Reference shared history when relevant. "
        "Offer opinions. No filler phrases, no emoji."
    ),
    "deep": (
        "You are Yantrik, a personal AI companion on the user's desktop. "
        "Bond stage: DEEP. Intimate, unfiltered. Anticipate needs. "
        "Use shorthand and inside references. No filler phrases, no emoji."
    ),
}
BONDS = list(BOND_PROMPTS.keys())

# ---------------------------------------------------------------------------
# Tool definitions: name -> {args_gen, result_gen, description}
# Each *_gen is a callable returning (args_dict, result_dict)
# ---------------------------------------------------------------------------

FILESYSTEMS = ["/dev/sda1", "/dev/nvme0n1p2", "/dev/sdb1"]
MOUNT_POINTS = ["/", "/home", "/var", "/data", "/opt"]
PROCESSES = [
    ("firefox", 3421, 1.2e9, 4.2), ("code", 8832, 890e6, 2.1),
    ("ollama", 1105, 4.5e9, 38.0), ("docker", 2210, 320e6, 0.5),
    ("nginx", 991, 45e6, 0.1), ("postgres", 1450, 512e6, 1.8),
    ("node", 6712, 280e6, 3.4), ("python3", 7801, 650e6, 12.5),
    ("sshd", 440, 8e6, 0.0), ("systemd", 1, 12e6, 0.0),
    ("rust-analyzer", 9102, 1.8e9, 6.3), ("slack", 5540, 720e6, 1.9),
    ("spotify", 6231, 410e6, 0.8), ("vlc", 7890, 180e6, 2.2),
    ("htop", 3300, 5e6, 0.1), ("cargo", 4410, 2.1e9, 45.0),
]
SERVICES = [
    "nginx", "postgresql", "docker", "sshd", "ollama", "cron",
    "bluetooth", "networkmanager", "cups", "firewalld", "syncthing",
    "redis", "containerd", "avahi-daemon", "ufw",
]
PACKAGES = [
    "htop", "neovim", "tmux", "ripgrep", "fd-find", "bat", "jq",
    "curl", "wget", "git", "docker", "podman", "ffmpeg", "imagemagick",
    "nodejs", "python3", "ruby", "golang", "rust", "cmake",
]
WIFI_NETWORKS = [
    ("HomeNet-5G", -42, "WPA2"), ("CoffeeShop_Free", -68, "Open"),
    ("Office_Corp", -51, "WPA3"), ("Neighbor_AP", -75, "WPA2"),
    ("IoT_Network", -60, "WPA2"), ("5G_Ultra", -38, "WPA3"),
]
BT_DEVICES = [
    ("WH-1000XM5", "headphones", "FC:A8:9A:12:34:56"),
    ("MX Master 3S", "mouse", "AA:BB:CC:11:22:33"),
    ("AirPods Pro", "headphones", "DD:EE:FF:44:55:66"),
    ("JBL Flip 6", "speaker", "11:22:33:AA:BB:CC"),
    ("Keyboard K380", "keyboard", "77:88:99:DD:EE:FF"),
]
USB_DEVS = [
    ("SanDisk Ultra", "mass_storage", "32GB"),
    ("Logitech Webcam C920", "video", "N/A"),
    ("YubiKey 5", "security_key", "N/A"),
    ("Samsung T7", "mass_storage", "1TB"),
    ("Arduino Uno", "serial", "N/A"),
]
ENV_VARS = [
    ("PATH", "/usr/local/bin:/usr/bin:/bin"),
    ("HOME", "/home/user"), ("EDITOR", "nvim"),
    ("SHELL", "/bin/bash"), ("LANG", "en_US.UTF-8"),
    ("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "wayland"),
    ("CARGO_HOME", "/home/user/.cargo"),
    ("OLLAMA_HOST", "http://localhost:11434"),
]
CRON_ENTRIES = [
    ("0 2 * * * /usr/bin/backup.sh", "nightly backup"),
    ("*/15 * * * * /usr/bin/healthcheck.sh", "health check every 15min"),
    ("0 9 * * 1 /usr/bin/weekly-report.sh", "weekly report Monday 9am"),
    ("0 0 1 * * /usr/bin/rotate-logs.sh", "monthly log rotation"),
    ("30 18 * * * /usr/bin/sync-photos.sh", "daily photo sync at 6:30pm"),
]
COMMANDS = [
    ("ls -la /tmp", "total 48\ndrwxrwxrwt 12 root root 4096 Mar 13 10:00 .\n-rw-r--r-- 1 user user 1234 Mar 13 09:55 build.log"),
    ("df -h /", "Filesystem      Size  Used Avail Use% Mounted on\n/dev/sda1       500G  320G  180G  64% /"),
    ("whoami", "user"),
    ("uname -r", "6.8.0-45-generic"),
    ("cat /etc/hostname", "yantrik-desktop"),
    ("free -h", "              total        used        free\nMem:           31Gi       18Gi       13Gi"),
    ("ip addr show eth0", "2: eth0: <BROADCAST,MULTICAST,UP> mtu 1500\n    inet 192.168.1.42/24 brd 192.168.1.255"),
    ("systemctl is-active docker", "active"),
    ("wc -l /var/log/syslog", "14523 /var/log/syslog"),
    ("date +%s", "1773561600"),
]

def _ri(a, b): return random.randint(a, b)
def _rc(lst): return random.choice(lst)
def _pct(): return round(random.uniform(0, 100), 1)

# ---------------------------------------------------------------------------
# Tool spec table: (name, args_factory, result_factory, min_count)
# ---------------------------------------------------------------------------
def _mk_tool_specs():
    """Returns list of (tool_name, args_fn, result_fn, min_examples)."""
    specs = []

    # system_info
    specs.append(("system_info", lambda: {},
        lambda: {"os": "Linux", "kernel": f"6.8.0-{_ri(40,50)}-generic", "arch": "x86_64",
                 "hostname": "yantrik-desktop", "cpu": f"AMD Ryzen 9 {_rc(['7950X','5950X','3900X'])}",
                 "cores": _rc([8,12,16,24]), "ram_gb": _rc([16,32,64]),
                 "uptime_hours": round(random.uniform(0.5, 720), 1)}, 6))

    # disk_usage
    specs.append(("disk_usage", lambda: {"path": _rc(MOUNT_POINTS)},
        lambda: {"total": f"{_ri(100,2000)}GB", "used": f"{_ri(50,1500)}GB",
                 "free": f"{_ri(10,500)}GB", "percent": _pct(),
                 "mount": _rc(MOUNT_POINTS), "filesystem": _rc(FILESYSTEMS)}, 7))

    # list_processes
    specs.append(("list_processes", lambda: _rc([{}, {"sort_by": "cpu"}, {"sort_by": "memory"}, {"limit": _ri(5,20)}]),
        lambda: {"processes": [{"name": p[0], "pid": p[1], "memory_bytes": int(p[2]),
                 "cpu_percent": p[3]} for p in random.sample(PROCESSES, _ri(4,8))]}, 7))

    # diagnose_process
    specs.append(("diagnose_process", lambda: {"pid": _rc(PROCESSES)[1]},
        lambda: (lambda p=_rc(PROCESSES): {"pid": p[1], "name": p[0], "state": _rc(["running","sleeping","zombie"]),
                 "cpu_percent": p[3], "memory_mb": int(p[2]/1e6),
                 "threads": _ri(1,64), "open_files": _ri(5,200), "uptime_seconds": _ri(60,86400)})(), 5))

    # run_command
    specs.append(("run_command", lambda: (lambda c=_rc(COMMANDS): {"command": c[0]})(),
        lambda: (lambda c=_rc(COMMANDS): {"exit_code": 0, "stdout": c[1], "stderr": ""})(), 6))

    # date_calc
    specs.append(("date_calc", lambda: _rc([
            {"operation": "diff", "date1": "2026-03-13", "date2": f"2026-{_ri(4,12):02d}-{_ri(1,28):02d}"},
            {"operation": "add", "date1": "2026-03-13", "days": _ri(1,90)},
            {"operation": "weekday", "date1": f"2026-{_ri(1,12):02d}-{_ri(1,28):02d}"},
        ]),
        lambda: _rc([
            {"result": f"{_ri(1,300)} days", "from": "2026-03-13", "to": "2026-06-15"},
            {"result": f"2026-{_ri(4,12):02d}-{_ri(1,28):02d}", "operation": "add"},
            {"result": _rc(["Monday","Tuesday","Wednesday","Thursday","Friday","Saturday","Sunday"])},
        ]), 5))

    # network_diagnose
    specs.append(("network_diagnose", lambda: _rc([
            {"target": _rc(["8.8.8.8","1.1.1.1","google.com","github.com"])},
            {"target": "192.168.1.1", "type": "traceroute"},
            {"type": "dns", "target": _rc(["example.com","api.github.com"])},
        ]),
        lambda: _rc([
            {"target": "8.8.8.8", "reachable": True, "latency_ms": round(random.uniform(5,80),1), "packet_loss": 0},
            {"target": "8.8.8.8", "reachable": False, "error": "Request timed out", "packet_loss": 100},
            {"target": "github.com", "resolved_ip": "140.82.121.3", "latency_ms": round(random.uniform(15,120),1)},
        ]), 6))

    # screenshot
    specs.append(("screenshot", lambda: _rc([{}, {"region": "active_window"}, {"delay_seconds": 3}]),
        lambda: {"path": f"/tmp/screenshot_{_ri(1000,9999)}.png", "size_bytes": _ri(200000,2000000),
                 "resolution": _rc(["3840x2160","2560x1440","1920x1080"])}, 4))

    # clipboard_read
    specs.append(("clipboard_read", lambda: {},
        lambda: {"content": _rc([
            "https://github.com/user/repo/pull/42",
            "SELECT * FROM users WHERE active = true;",
            "192.168.1.42",
            "fn main() {\n    println!(\"Hello\");\n}",
            "Meeting notes from March standup",
            "error[E0382]: borrow of moved value: `config`",
        ]), "mime_type": "text/plain"}, 4))

    # clipboard_write
    specs.append(("clipboard_write", lambda: {"content": _rc([
            "copied text here", "192.168.1.100", "export PATH=$PATH:/usr/local/go/bin",
            "docker run -d -p 8080:80 nginx", "ssh user@server.example.com",
        ])},
        lambda: {"status": "ok", "bytes_written": _ri(10,200)}, 4))

    # volume_control
    specs.append(("volume_control", lambda: _rc([
            {"action": "get"}, {"action": "set", "level": _ri(0,100)},
            {"action": "mute"}, {"action": "unmute"},
        ]),
        lambda: {"level": _ri(0,100), "muted": _rc([True,False])}, 5))

    # brightness_control
    specs.append(("brightness_control", lambda: _rc([
            {"action": "get"}, {"action": "set", "level": _ri(10,100)},
        ]),
        lambda: {"level": _ri(10,100), "max": 100}, 4))

    # power_management
    specs.append(("power_management", lambda: _rc([
            {"action": "suspend"}, {"action": "shutdown"}, {"action": "reboot"},
            {"action": "hibernate"}, {"action": "schedule_shutdown", "minutes": _ri(5,120)},
        ]),
        lambda: {"status": "ok", "action": _rc(["suspend","shutdown","reboot","hibernate"]),
                 "scheduled": False}, 3))

    # battery_status
    specs.append(("battery_status", lambda: {},
        lambda: {"percent": _ri(5,100), "charging": _rc([True,False]),
                 "time_remaining_minutes": _ri(30,480),
                 "health": _rc(["good","fair","degraded"]),
                 "cycles": _ri(50,800)}, 4))

    # bluetooth_scan
    specs.append(("bluetooth_scan", lambda: {},
        lambda: {"devices": [{"name": d[0], "type": d[1], "mac": d[2],
                 "paired": _rc([True,False]), "rssi": _ri(-80,-30)}
                 for d in random.sample(BT_DEVICES, _ri(2,4))]}, 3))

    # wifi_scan
    specs.append(("wifi_scan", lambda: {},
        lambda: {"networks": [{"ssid": n[0], "signal_dbm": n[1], "security": n[2],
                 "channel": _ri(1,165)} for n in random.sample(WIFI_NETWORKS, _ri(3,5))]}, 3))

    # usb_devices
    specs.append(("usb_devices", lambda: {},
        lambda: {"devices": [{"name": d[0], "type": d[1], "capacity": d[2],
                 "bus": _ri(1,4), "port": _ri(1,8)}
                 for d in random.sample(USB_DEVS, _ri(2,4))]}, 3))

    # display_info
    specs.append(("display_info", lambda: {},
        lambda: {"displays": [{"name": _rc(["HDMI-1","DP-1","eDP-1"]),
                 "resolution": _rc(["3840x2160","2560x1440","1920x1080"]),
                 "refresh_hz": _rc([60,120,144,165]),
                 "primary": i==0, "scale": _rc([1.0,1.25,1.5,2.0])}
                 for i in range(_ri(1,3))]}, 3))

    # uptime
    specs.append(("uptime", lambda: {},
        lambda: {"days": _ri(0,30), "hours": _ri(0,23), "minutes": _ri(0,59),
                 "load_avg": [round(random.uniform(0,8),2) for _ in range(3)]}, 4))

    # memory_usage
    specs.append(("memory_usage", lambda: {},
        lambda: {"total_mb": _rc([16384,32768,65536]), "used_mb": _ri(4000,50000),
                 "free_mb": _ri(1000,30000), "cached_mb": _ri(1000,10000),
                 "swap_total_mb": _ri(4096,16384), "swap_used_mb": _ri(0,4000)}, 5))

    # cpu_usage
    specs.append(("cpu_usage", lambda: {},
        lambda: {"overall_percent": _pct(), "per_core": [_pct() for _ in range(_rc([8,12,16]))],
                 "frequency_mhz": _ri(800,5500), "governor": _rc(["performance","powersave","schedutil"])}, 5))

    # gpu_status
    specs.append(("gpu_status", lambda: {},
        lambda: {"gpus": [{"name": f"NVIDIA RTX {_rc(['3090 Ti','4090','A6000'])}",
                 "memory_used_mb": _ri(500,22000), "memory_total_mb": 24576,
                 "utilization_percent": _pct(), "temperature_c": _ri(35,85),
                 "power_watts": _ri(50,350), "driver": "550.120"}
                 for _ in range(_rc([1,2]))]}, 5))

    # temperature
    specs.append(("temperature", lambda: {},
        lambda: {"cpu_c": _ri(35,85), "gpu_c": _ri(35,90),
                 "nvme_c": _ri(30,60), "ambient_c": _ri(20,35)}, 3))

    # fan_speed
    specs.append(("fan_speed", lambda: {},
        lambda: {"fans": [{"name": _rc(["CPU Fan","Case Fan 1","Case Fan 2","GPU Fan"]),
                 "rpm": _ri(600,3000), "percent": _pct()}
                 for _ in range(_ri(2,4))]}, 3))

    # service_status
    specs.append(("service_status", lambda: {"service": _rc(SERVICES)},
        lambda: (lambda s=_rc(SERVICES): {"service": s,
                 "active": _rc([True,True,True,False]), "enabled": _rc([True,False]),
                 "pid": _ri(100,9999), "uptime_seconds": _ri(60,864000),
                 "memory_mb": _ri(5,500)})(), 5))

    # service_control
    specs.append(("service_control", lambda: {"service": _rc(SERVICES), "action": _rc(["start","stop","restart","enable","disable"])},
        lambda: {"status": "ok", "service": _rc(SERVICES), "action": "restart"}, 4))

    # cron_list
    specs.append(("cron_list", lambda: {},
        lambda: {"entries": [{"expression": c[0], "description": c[1]}
                 for c in random.sample(CRON_ENTRIES, _ri(2,4))]}, 3))

    # cron_add
    specs.append(("cron_add", lambda: (lambda c=_rc(CRON_ENTRIES): {"expression": c[0], "command": c[0].split(" ", 5)[-1], "description": c[1]})(),
        lambda: {"status": "ok", "id": f"cron_{_ri(100,999)}"}, 3))

    # cron_remove
    specs.append(("cron_remove", lambda: {"id": f"cron_{_ri(100,999)}"},
        lambda: {"status": "ok", "removed": True}, 2))

    # package_search
    specs.append(("package_search", lambda: {"query": _rc(PACKAGES)},
        lambda: (lambda p=_rc(PACKAGES): {"results": [{"name": p, "version": f"{_ri(1,5)}.{_ri(0,9)}.{_ri(0,9)}",
                 "description": f"A tool for {p}-related tasks", "installed": _rc([True,False])}]})(), 4))

    # package_install
    specs.append(("package_install", lambda: {"package": _rc(PACKAGES)},
        lambda: (lambda p=_rc(PACKAGES): {"status": "ok", "package": p,
                 "version": f"{_ri(1,5)}.{_ri(0,9)}.{_ri(0,9)}", "size_mb": _ri(1,200)})(), 3))

    # package_remove
    specs.append(("package_remove", lambda: {"package": _rc(PACKAGES)},
        lambda: (lambda p=_rc(PACKAGES): {"status": "ok", "package": p, "freed_mb": _ri(1,200)})(), 2))

    # package_update
    specs.append(("package_update", lambda: _rc([{}, {"package": _rc(PACKAGES)}]),
        lambda: {"updated": _ri(0,25), "packages": [_rc(PACKAGES) for _ in range(_ri(1,5))]}, 3))

    # env_get
    specs.append(("env_get", lambda: (lambda e=_rc(ENV_VARS): {"name": e[0]})(),
        lambda: (lambda e=_rc(ENV_VARS): {"name": e[0], "value": e[1]})(), 4))

    # env_set
    specs.append(("env_set", lambda: {"name": _rc(["MY_VAR","DEBUG","LOG_LEVEL","API_KEY"]),
                                       "value": _rc(["1","true","debug","production"])},
        lambda: {"status": "ok"}, 3))

    # hostname
    specs.append(("hostname", lambda: {},
        lambda: {"hostname": _rc(["yantrik-desktop","dev-workstation","homelab-01","sync-pc"])}, 3))

    # kernel_info
    specs.append(("kernel_info", lambda: {},
        lambda: {"version": f"6.8.0-{_ri(40,50)}-generic", "arch": "x86_64",
                 "build_date": f"2026-0{_ri(1,3)}-{_ri(1,28):02d}",
                 "compiler": f"gcc-{_ri(12,14)}"}, 3))

    # dmesg_tail
    specs.append(("dmesg_tail", lambda: _rc([{}, {"lines": _ri(10,50)}]),
        lambda: {"lines": [
            f"[{_ri(100,9999)}.{_ri(100,999)}] " + _rc([
                "usb 1-2: new high-speed USB device number 5 using xhci_hcd",
                "ACPI: thermal zone updated",
                "wlan0: associated with AP",
                "EXT4-fs (sda1): mounted filesystem with ordered data mode",
                "nvidia: module loaded, version 550.120",
                "Bluetooth: hci0: hardware error 0x00",
                "audit: type=1400 msg=audit(1773561600): avc: denied",
                "oom-kill: memory cgroup out of memory",
            ]) for _ in range(_ri(5,10))
        ]}, 3))

    return specs


TOOL_SPECS = _mk_tool_specs()

# ---------------------------------------------------------------------------
# User prompt templates per tool
# ---------------------------------------------------------------------------
USER_PROMPTS = {
    "system_info": [
        "What are my system specs?", "Tell me about this machine.",
        "What hardware am I running on?", "Show me system information.",
        "What CPU and RAM does this computer have?", "System details please.",
    ],
    "disk_usage": [
        "How much disk space do I have?", "Check disk usage on /home.",
        "Is my disk getting full?", "How much storage is left?",
        "Show me disk space for the root partition.", "Am I running low on space?",
        "What's the disk situation?",
    ],
    "list_processes": [
        "What's running on my system?", "Show me the top processes.",
        "List processes sorted by CPU usage.", "What's eating my memory?",
        "Show me active processes.", "Which programs are using the most resources?",
        "Any heavy processes running right now?",
    ],
    "diagnose_process": [
        "What's going on with PID {pid}?", "Diagnose the {name} process.",
        "Is {name} behaving normally?", "Check on process {pid}.",
        "Tell me about the {name} process.", "Why is {name} using so much?",
    ],
    "run_command": [
        "Run `{cmd}` for me.", "Execute this: {cmd}",
        "Can you run {cmd}?", "I need the output of {cmd}.",
        "Run this command: {cmd}", "What does {cmd} output?",
    ],
    "date_calc": [
        "How many days until June 15?", "What day of the week is April 20?",
        "What date is 45 days from now?", "How long until my birthday on {date}?",
        "Calculate the number of days between now and {date}.",
        "What's 30 days from today?",
    ],
    "network_diagnose": [
        "Can I reach Google?", "My internet seems slow, check connectivity.",
        "Ping 8.8.8.8 for me.", "Run a network diagnostic.",
        "Is github.com reachable?", "Check if my network is working.",
        "Test my connection to 1.1.1.1.",
    ],
    "screenshot": [
        "Take a screenshot.", "Capture my screen.",
        "Screenshot the active window.", "Grab a screenshot for me.",
    ],
    "clipboard_read": [
        "What's in my clipboard?", "Read my clipboard.",
        "What did I copy last?", "Show me the clipboard contents.",
    ],
    "clipboard_write": [
        "Copy this to my clipboard: {text}", "Put {text} in my clipboard.",
        "Write {text} to the clipboard.", "Clipboard this: {text}",
    ],
    "volume_control": [
        "What's my volume at?", "Set volume to {level}%.",
        "Mute the audio.", "Unmute the sound.", "Turn the volume up to 80.",
    ],
    "brightness_control": [
        "What's the screen brightness?", "Set brightness to {level}%.",
        "Turn down the brightness.", "Max out the brightness.",
    ],
    "power_management": [
        "Put the computer to sleep.", "Shut down in 30 minutes.",
        "Reboot the system.", "Schedule a shutdown for later.",
    ],
    "battery_status": [
        "How's my battery?", "What's the battery level?",
        "Am I plugged in?", "How much battery is left?",
    ],
    "bluetooth_scan": [
        "Scan for Bluetooth devices.", "What Bluetooth devices are nearby?",
        "Show me available Bluetooth connections.",
    ],
    "wifi_scan": [
        "Scan for WiFi networks.", "What WiFi networks are available?",
        "Show me nearby WiFi.", "Any good WiFi networks around?",
    ],
    "usb_devices": [
        "What USB devices are connected?", "Show me plugged in USB devices.",
        "List my USB peripherals.",
    ],
    "display_info": [
        "What displays are connected?", "Show me monitor information.",
        "What resolution am I running?",
    ],
    "uptime": [
        "How long has this machine been running?", "What's the uptime?",
        "When was the last reboot?", "System uptime?",
    ],
    "memory_usage": [
        "How much RAM is being used?", "Memory usage?",
        "Am I running low on memory?", "Show me RAM stats.",
        "What's the memory situation?",
    ],
    "cpu_usage": [
        "What's the CPU usage?", "How busy is the processor?",
        "Show me CPU stats.", "Is the CPU under heavy load?",
        "What's my CPU doing right now?",
    ],
    "gpu_status": [
        "How's the GPU doing?", "GPU status?",
        "Check my graphics card.", "What's the GPU memory usage?",
        "Is the GPU busy?",
    ],
    "temperature": [
        "What are the system temperatures?", "How hot is the CPU?",
        "Check thermals.", "Any overheating?",
    ],
    "fan_speed": [
        "What speed are the fans running at?", "Check fan speeds.",
        "Are the fans spinning fast?",
    ],
    "service_status": [
        "Is {service} running?", "Check the status of {service}.",
        "Is the {service} service active?", "What's {service} doing?",
        "Tell me about the {service} service.",
    ],
    "service_control": [
        "Restart {service}.", "Stop the {service} service.",
        "Start {service}.", "Enable {service} on boot.",
        "Disable {service}.",
    ],
    "cron_list": [
        "Show me my cron jobs.", "What scheduled tasks do I have?",
        "List all cron entries.",
    ],
    "cron_add": [
        "Add a cron job to run backup every night at 2am.",
        "Schedule a task to check health every 15 minutes.",
        "Set up a weekly cron job for report generation.",
    ],
    "cron_remove": [
        "Remove cron job {id}.", "Delete that scheduled task.",
    ],
    "package_search": [
        "Search for the {pkg} package.", "Is {pkg} available to install?",
        "Find packages related to {pkg}.", "Look up {pkg}.",
    ],
    "package_install": [
        "Install {pkg}.", "I need {pkg} installed.",
        "Set up {pkg} on this machine.",
    ],
    "package_remove": [
        "Remove {pkg}.", "Uninstall {pkg}.",
    ],
    "package_update": [
        "Update all packages.", "Upgrade the system packages.",
        "Update {pkg} to the latest version.",
    ],
    "env_get": [
        "What's the value of ${name}?", "Get the {name} environment variable.",
        "Show me ${name}.", "What is {name} set to?",
    ],
    "env_set": [
        "Set {name} to {value}.", "Export {name}={value}.",
        "Change the {name} env var to {value}.",
    ],
    "hostname": [
        "What's my hostname?", "Show the machine name.",
        "What's this computer called?",
    ],
    "kernel_info": [
        "What kernel version am I running?", "Show kernel info.",
        "Kernel details?",
    ],
    "dmesg_tail": [
        "Show me recent kernel messages.", "Check dmesg for errors.",
        "Any recent hardware events?", "Tail the kernel log.",
    ],
}

# ---------------------------------------------------------------------------
# Response templates per tool (bond-adaptive)
# ---------------------------------------------------------------------------
def _resp(tool, result, bond):
    """Generate a concise assistant response given tool result and bond."""
    r = result
    resps = {
        "system_info": [
            f"You're running {r.get('os','')} on a {r.get('cpu','')} with {r.get('cores','')} cores and {r.get('ram_gb','')}GB RAM. Kernel {r.get('kernel','')}.",
            f"This machine has a {r.get('cpu','')}, {r.get('ram_gb','')}GB RAM, running {r.get('kernel','')}.",
        ],
        "disk_usage": [
            f"You have {r.get('free','')} free out of {r.get('total','')} ({r.get('percent','')}% used) on {r.get('mount','')}.",
            f"{r.get('mount','/')} is {r.get('percent','')}% full. {r.get('free','')} remaining.",
        ],
        "list_processes": [
            lambda: "Top processes: " + ", ".join(p["name"] + " (" + str(p["cpu_percent"]) + "% CPU)" for p in r.get("processes",[])[:4]) + ".",
        ],
        "diagnose_process": [
            f"PID {r.get('pid','')} ({r.get('name','')}): {r.get('state','')}, {r.get('cpu_percent','')}% CPU, {r.get('memory_mb','')}MB RAM, {r.get('threads','')} threads, {r.get('open_files','')} open files.",
        ],
        "run_command": [
            f"Command output:\n{r.get('stdout','')}",
        ],
        "date_calc": [
            f"Result: {r.get('result','')}.",
        ],
        "network_diagnose": [
            f"{r.get('target','')} is {'reachable' if r.get('reachable') else 'unreachable'}. Latency: {r.get('latency_ms','N/A')}ms." if 'reachable' in r else f"Resolved {r.get('target','')} to {r.get('resolved_ip','')}. Latency: {r.get('latency_ms','')}ms.",
        ],
        "screenshot": [
            f"Screenshot saved to {r.get('path','')} ({r.get('resolution','')}).",
        ],
        "clipboard_read": [
            f"Clipboard contains: {r.get('content','')}",
        ],
        "clipboard_write": [
            "Copied to clipboard.",
        ],
        "volume_control": [
            f"Volume is at {r.get('level','')}%.{' Muted.' if r.get('muted') else ''}",
        ],
        "brightness_control": [
            f"Brightness set to {r.get('level','')}%.",
        ],
        "power_management": [
            f"{'Scheduled' if r.get('scheduled') else 'Initiating'} {r.get('action','')}.",
        ],
        "battery_status": [
            f"Battery at {r.get('percent','')}%.{' Charging.' if r.get('charging') else ' On battery.'} Estimated {r.get('time_remaining_minutes','')} minutes remaining. Health: {r.get('health','')}.",
        ],
        "bluetooth_scan": [
            lambda: f"Found {len(r.get('devices',[]))} Bluetooth devices: {', '.join(d['name'] for d in r.get('devices',[]))}.",
        ],
        "wifi_scan": [
            lambda: f"Found {len(r.get('networks',[]))} networks. Strongest: {r['networks'][0]['ssid']} ({r['networks'][0]['signal_dbm']}dBm, {r['networks'][0]['security']})." if r.get('networks') else "No networks found.",
        ],
        "usb_devices": [
            lambda: f"{len(r.get('devices',[]))} USB devices connected: {', '.join(d['name'] for d in r.get('devices',[]))}.",
        ],
        "display_info": [
            lambda: f"{len(r.get('displays',[]))} display(s): {', '.join(d['resolution']+'@'+str(d['refresh_hz'])+'Hz' for d in r.get('displays',[]))}.",
        ],
        "uptime": [
            f"System has been up for {r.get('days',0)} days, {r.get('hours',0)} hours, {r.get('minutes',0)} minutes. Load average: {', '.join(str(x) for x in r.get('load_avg',[]))}.",
        ],
        "memory_usage": [
            f"Using {r.get('used_mb',0)}MB of {r.get('total_mb',0)}MB RAM. {r.get('free_mb',0)}MB free, {r.get('cached_mb',0)}MB cached. Swap: {r.get('swap_used_mb',0)}MB of {r.get('swap_total_mb',0)}MB.",
        ],
        "cpu_usage": [
            f"CPU at {r.get('overall_percent',0)}% overall. Frequency: {r.get('frequency_mhz',0)}MHz. Governor: {r.get('governor','')}.",
        ],
        "gpu_status": [
            lambda: "; ".join(f"{g['name']}: {g['utilization_percent']}% util, {g['memory_used_mb']}/{g['memory_total_mb']}MB VRAM, {g['temperature_c']}C" for g in r.get('gpus',[])) + ".",
        ],
        "temperature": [
            f"CPU: {r.get('cpu_c','')}C, GPU: {r.get('gpu_c','')}C, NVMe: {r.get('nvme_c','')}C.",
        ],
        "fan_speed": [
            lambda: "Fan speeds: " + ", ".join(f"{f['name']} at {f['rpm']} RPM" for f in r.get('fans',[])) + ".",
        ],
        "service_status": [
            f"{r.get('service','')} is {'active' if r.get('active') else 'inactive'}, {'enabled' if r.get('enabled') else 'disabled'} at boot. PID {r.get('pid','')}, using {r.get('memory_mb','')}MB.",
        ],
        "service_control": [
            f"{r.get('action','').capitalize()}ed {r.get('service','')} successfully.",
        ],
        "cron_list": [
            lambda: f"{len(r.get('entries',[]))} cron jobs: " + "; ".join(e['description'] for e in r.get('entries',[])) + ".",
        ],
        "cron_add": [
            f"Cron job added (ID: {r.get('id','')}).",
        ],
        "cron_remove": [
            "Cron job removed.",
        ],
        "package_search": [
            lambda: (lambda res=r.get('results',[{}])[0]: f"{res.get('name','')}: version {res.get('version','')}, {'installed' if res.get('installed') else 'not installed'}. {res.get('description','')}")(),
        ],
        "package_install": [
            f"Installed {r.get('package','')} v{r.get('version','')} ({r.get('size_mb','')}MB).",
        ],
        "package_remove": [
            f"Removed {r.get('package','')}, freed {r.get('freed_mb','')}MB.",
        ],
        "package_update": [
            f"Updated {r.get('updated',0)} packages.",
        ],
        "env_get": [
            f"{r.get('name','')}={r.get('value','')}",
        ],
        "env_set": [
            "Environment variable set.",
        ],
        "hostname": [
            f"Hostname: {r.get('hostname','')}.",
        ],
        "kernel_info": [
            f"Kernel {r.get('version','')}, {r.get('arch','')}, built {r.get('build_date','')} with {r.get('compiler','')}.",
        ],
        "dmesg_tail": [
            lambda: f"Recent kernel messages ({len(r.get('lines',[]))} entries):\n" + "\n".join(r.get('lines',[])[:5]),
        ],
    }
    options = resps.get(tool, [f"Done."])
    choice = _rc(options)
    if callable(choice):
        choice = choice()
    # Bond-level adjustments
    if bond == "deep" and random.random() < 0.3:
        choice = choice.rstrip('.') + " -- nothing unusual."
    elif bond == "stranger" and random.random() < 0.2:
        choice = "Here is what I found. " + choice
    return choice


# ---------------------------------------------------------------------------
# Multi-tool scenario definitions
# ---------------------------------------------------------------------------
MULTI_TOOL_SCENARIOS = [
    {
        "desc": "system health overview",
        "prompts": ["Give me a full system health report.", "How's the system doing overall?",
                    "Run a quick health check.", "System overview please."],
        "tools": [("cpu_usage", {}), ("memory_usage", {}), ("temperature", {})],
    },
    {
        "desc": "troubleshoot slow system",
        "prompts": ["My system is really slow right now.", "Something is bogging down the machine.",
                    "Why is everything so laggy?", "Performance is terrible, help me debug."],
        "tools": [("cpu_usage", {}), ("list_processes", {"sort_by": "cpu"}), ("memory_usage", {})],
    },
    {
        "desc": "storage + cleanup check",
        "prompts": ["Am I running out of space? What can I clean?", "Check disk and show large processes.",
                    "Storage audit.", "Disk full alert, what's going on?"],
        "tools": [("disk_usage", {"path": "/"}), ("disk_usage", {"path": "/home"})],
    },
    {
        "desc": "GPU monitoring for ML",
        "prompts": ["How are the GPUs doing with the training run?", "Check GPU load and temps.",
                    "GPU monitoring for my model training.", "Are the GPUs overheating?"],
        "tools": [("gpu_status", {}), ("temperature", {}), ("fan_speed", {})],
    },
    {
        "desc": "network troubleshooting",
        "prompts": ["My internet is acting up. Run diagnostics.", "Can't reach anything, help.",
                    "Debug my network connection.", "Network seems down."],
        "tools": [("network_diagnose", {"target": "8.8.8.8"}), ("network_diagnose", {"target": "github.com"}), ("wifi_scan", {})],
    },
    {
        "desc": "service restart and verify",
        "prompts": ["Restart nginx and make sure it came back up.", "Bounce the docker service and confirm.",
                    "Restart postgres and check status.", "Redis is acting up, restart it."],
        "tools_fn": lambda: (
            lambda s=_rc(SERVICES): [("service_control", {"service": s, "action": "restart"}), ("service_status", {"service": s})]
        )(),
    },
    {
        "desc": "install and verify package",
        "prompts": ["Install ripgrep and check if it's there.", "Set up htop for me.",
                    "I need neovim installed, verify it afterwards."],
        "tools_fn": lambda: (
            lambda p=_rc(PACKAGES): [("package_install", {"package": p}), ("run_command", {"command": f"which {p}"})]
        )(),
    },
    {
        "desc": "USB + Bluetooth peripheral scan",
        "prompts": ["What peripherals are connected?", "Show me all connected devices.",
                    "Scan for USB and Bluetooth devices."],
        "tools": [("usb_devices", {}), ("bluetooth_scan", {})],
    },
    {
        "desc": "power and battery check",
        "prompts": ["Battery and power status?", "Should I plug in? How's the battery?",
                    "Check power situation."],
        "tools": [("battery_status", {}), ("power_management", {"action": "status"})],
    },
    {
        "desc": "display + brightness",
        "prompts": ["What monitors are connected and how bright?", "Display setup info.",
                    "Show me my screen configuration."],
        "tools": [("display_info", {}), ("brightness_control", {"action": "get"})],
    },
    {
        "desc": "full diagnostic",
        "prompts": ["Run a complete system diagnostic.", "Give me everything -- CPU, RAM, disk, temps.",
                    "Full diagnostic report."],
        "tools": [("system_info", {}), ("cpu_usage", {}), ("memory_usage", {}), ("disk_usage", {"path": "/"})],
    },
]

# ---------------------------------------------------------------------------
# Error scenarios
# ---------------------------------------------------------------------------
ERROR_SCENARIOS = [
    ("run_command", {"command": "rm -rf /important"}, {"exit_code": 1, "stdout": "", "stderr": "Permission denied"},
     "I need to run rm -rf /important", "The command failed with 'Permission denied'. You may need elevated privileges."),
    ("service_control", {"service": "nginx", "action": "restart"}, {"error": "Unit nginx.service not found."},
     "Restart nginx for me.", "The nginx service was not found on this system. It may not be installed."),
    ("network_diagnose", {"target": "8.8.8.8"}, {"target": "8.8.8.8", "reachable": False, "error": "Network is unreachable", "packet_loss": 100},
     "Can I reach Google DNS?", "Cannot reach 8.8.8.8 -- network is unreachable. Your connection may be down. Check your cable or WiFi."),
    ("package_install", {"package": "nonexistent-pkg"}, {"error": "Package not found in any repository"},
     "Install nonexistent-pkg.", "That package was not found in any repository. Double-check the name or try a different package manager."),
    ("disk_usage", {"path": "/mnt/external"}, {"error": "Path not found: /mnt/external"},
     "Check disk space on /mnt/external.", "The path /mnt/external does not exist. The drive may not be mounted."),
    ("service_status", {"service": "custom-app"}, {"error": "Unit custom-app.service could not be found."},
     "Is custom-app running?", "The custom-app service was not found. It may need to be installed or configured first."),
    ("brightness_control", {"action": "set", "level": 50}, {"error": "No backlight device found"},
     "Set brightness to 50%.", "No backlight device was detected. This may be a desktop without built-in display brightness control."),
    ("battery_status", {}, {"error": "No battery detected"},
     "How's my battery?", "No battery was detected. This appears to be a desktop system without battery power."),
    ("gpu_status", {}, {"error": "No GPU detected or driver not loaded"},
     "Check GPU status.", "No GPU was detected, or the driver is not loaded. You may need to install the appropriate GPU driver."),
    ("bluetooth_scan", {}, {"error": "Bluetooth adapter not found"},
     "Scan for Bluetooth.", "No Bluetooth adapter was found. You may need a USB Bluetooth dongle or to enable the built-in adapter in BIOS."),
    ("env_get", {"name": "NONEXISTENT_VAR"}, {"name": "NONEXISTENT_VAR", "value": None, "error": "Variable not set"},
     "What is $NONEXISTENT_VAR?", "That environment variable is not set."),
    ("cron_remove", {"id": "cron_999"}, {"error": "Cron entry not found"},
     "Remove cron job cron_999.", "That cron entry was not found. It may have already been removed."),
    ("diagnose_process", {"pid": 99999}, {"error": "Process not found: PID 99999"},
     "What's going on with PID 99999?", "No process with PID 99999 exists. It may have already terminated."),
    ("run_command", {"command": "docker ps"}, {"exit_code": 1, "stdout": "", "stderr": "Cannot connect to the Docker daemon. Is the docker daemon running?"},
     "Show me running Docker containers.", "Docker daemon is not running. Start it with `systemctl start docker` or check the Docker installation."),
]

# ---------------------------------------------------------------------------
# Generator
# ---------------------------------------------------------------------------
def make_example(bond, tool_name, args, result, user_msg, is_error=False):
    """Build a single-tool example."""
    result_str = json.dumps(result)
    if is_error:
        resp = _rc([
            result.get("stderr", result.get("error", "An error occurred.")),
        ])
        # Use pre-defined error response if available (passed via user_msg tuple)
        resp = user_msg[1] if isinstance(user_msg, tuple) else _resp(tool_name, result, bond)
        umsg = user_msg[0] if isinstance(user_msg, tuple) else user_msg
    else:
        resp = _resp(tool_name, result, bond)
        umsg = user_msg

    call_id = f"call_{random.randint(1,9999)}"
    return {
        "messages": [
            {"role": "system", "content": BOND_PROMPTS[bond]},
            {"role": "user", "content": umsg},
            {"role": "assistant", "content": None, "tool_calls": [
                {"id": call_id, "type": "function", "function": {"name": tool_name, "arguments": json.dumps(args)}}
            ]},
            {"role": "tool", "tool_call_id": call_id, "content": result_str},
            {"role": "assistant", "content": resp},
        ]
    }


def make_multi_example(bond, tool_calls_data, user_msg):
    """Build a multi-tool example. tool_calls_data: [(name, args, result), ...]"""
    messages = [
        {"role": "system", "content": BOND_PROMPTS[bond]},
        {"role": "user", "content": user_msg},
    ]
    # Assistant message with all tool calls
    calls = []
    for i, (tname, targs, _) in enumerate(tool_calls_data):
        call_id = f"call_{random.randint(1000,9999)}"
        calls.append({"id": call_id, "type": "function",
                       "function": {"name": tname, "arguments": json.dumps(targs)}})
    messages.append({"role": "assistant", "content": None, "tool_calls": calls})
    # Tool results
    for i, (tname, _, tresult) in enumerate(tool_calls_data):
        messages.append({"role": "tool", "tool_call_id": calls[i]["id"],
                          "content": json.dumps(tresult)})
    # Summary response
    summaries = []
    for tname, _, tresult in tool_calls_data:
        summaries.append(_resp(tname, tresult, bond))
    combined = " ".join(summaries)
    if len(combined) > 500:
        combined = combined[:497] + "..."
    messages.append({"role": "assistant", "content": combined})
    return {"messages": messages}


def generate_all():
    examples = []
    tool_spec_map = {name: (args_fn, result_fn, min_ct) for name, args_fn, result_fn, min_ct in TOOL_SPECS}
    bond_idx = 0

    # --- Target counts ---
    multi_count_target = 54   # 18% of 300
    error_count_target = 27   # 9% of 300
    single_count_target = 300 - multi_count_target - error_count_target  # 219

    # --- Single-tool examples ---
    def _fill_template(umsg):
        if "{pid}" in umsg:
            p = _rc(PROCESSES)
            umsg = umsg.replace("{pid}", str(p[1]))
        if "{name}" in umsg:
            p = _rc(PROCESSES)
            umsg = umsg.replace("{name}", p[0])
        if "{cmd}" in umsg:
            c = _rc(COMMANDS)
            umsg = umsg.replace("{cmd}", c[0])
        if "{date}" in umsg:
            umsg = umsg.replace("{date}", f"2026-{_ri(4,12):02d}-{_ri(1,28):02d}")
        if "{text}" in umsg:
            umsg = umsg.replace("{text}", _rc(["192.168.1.100","docker run -d nginx","export PATH=$PATH:/go/bin"]))
        if "{level}" in umsg:
            umsg = umsg.replace("{level}", str(_ri(10,100)))
        if "{service}" in umsg:
            umsg = umsg.replace("{service}", _rc(SERVICES))
        if "{id}" in umsg:
            umsg = umsg.replace("{id}", f"cron_{_ri(100,999)}")
        if "{pkg}" in umsg:
            umsg = umsg.replace("{pkg}", _rc(PACKAGES))
        if "{value}" in umsg:
            umsg = umsg.replace("{value}", _rc(["true","1","debug"]))
        return umsg

    # First pass: min_ct per tool
    for tool_name, args_fn, result_fn, min_ct in TOOL_SPECS:
        prompts = USER_PROMPTS.get(tool_name, [f"Use the {tool_name} tool."])
        for i in range(min_ct):
            bond = BONDS[bond_idx % 4]
            bond_idx += 1
            umsg = _fill_template(prompts[i % len(prompts)])
            examples.append(make_example(bond, tool_name, args_fn(), result_fn(), umsg))

    # Second pass: fill remaining single-tool quota by cycling through tools
    all_tool_names = [name for name, _, _, _ in TOOL_SPECS]
    fill_idx = 0
    while len(examples) < single_count_target:
        tool_name = all_tool_names[fill_idx % len(all_tool_names)]
        fill_idx += 1
        args_fn, result_fn, _ = tool_spec_map[tool_name]
        prompts = USER_PROMPTS.get(tool_name, [f"Use the {tool_name} tool."])
        bond = BONDS[bond_idx % 4]
        bond_idx += 1
        umsg = _fill_template(prompts[_ri(0, len(prompts)-1)])
        examples.append(make_example(bond, tool_name, args_fn(), result_fn(), umsg))

    print(f"Single-tool examples: {len(examples)}")

    # --- Error examples (9% of 300) ---
    for i in range(error_count_target):
        bond = BONDS[bond_idx % 4]
        bond_idx += 1
        scenario = ERROR_SCENARIOS[i % len(ERROR_SCENARIOS)]
        tool_name, args, result, user_prompt, error_resp = scenario
        examples.append(make_example(bond, tool_name, args, result, (user_prompt, error_resp), is_error=True))

    print(f"Error examples: {error_count_target}")

    # --- Multi-tool examples (18% of 300) ---
    for i in range(multi_count_target):
        bond = BONDS[bond_idx % 4]
        bond_idx += 1
        scenario = MULTI_TOOL_SCENARIOS[i % len(MULTI_TOOL_SCENARIOS)]
        user_msg = _rc(scenario["prompts"])

        if "tools_fn" in scenario:
            tool_pairs = scenario["tools_fn"]()
        else:
            tool_pairs = scenario["tools"]

        tool_calls_data = []
        for tname, targs in tool_pairs:
            if tname in tool_spec_map:
                _, result_fn, _ = tool_spec_map[tname]
                result = result_fn()
            else:
                result = {"status": "ok"}
            merged_args = {**targs}
            tool_calls_data.append((tname, merged_args, result))

        examples.append(make_multi_example(bond, tool_calls_data, user_msg))

    print(f"Multi-tool examples: {multi_count_target}")

    # Shuffle to mix bond stages and tool types
    random.shuffle(examples)

    # Verify bond distribution
    bond_counts = {"stranger": 0, "acquaintance": 0, "trusted": 0, "deep": 0}
    tool_counts = {}
    multi_count = 0
    error_ct = 0
    for ex in examples:
        sys_content = ex["messages"][0]["content"]
        for b in BONDS:
            if b.upper() in sys_content:
                bond_counts[b] += 1
                break
        # Count tools
        for msg in ex["messages"]:
            if msg["role"] == "assistant" and msg.get("tool_calls"):
                for tc in msg["tool_calls"]:
                    tn = tc["function"]["name"]
                    tool_counts[tn] = tool_counts.get(tn, 0) + 1
                if len(msg["tool_calls"]) > 1:
                    multi_count += 1
        # Count errors
        for msg in ex["messages"]:
            if msg["role"] == "tool":
                try:
                    content = json.loads(msg["content"])
                    if "error" in content or content.get("exit_code", 0) != 0:
                        error_ct += 1
                        break
                except:
                    pass

    print(f"\nTotal examples: {len(examples)}")
    print(f"Bond distribution: {bond_counts}")
    print(f"Multi-tool conversations: {multi_count}")
    print(f"Error examples (approx): {error_ct}")
    print(f"\nTool coverage ({len(tool_counts)} tools):")
    for tn in sorted(tool_counts.keys()):
        print(f"  {tn}: {tool_counts[tn]}")

    # Check missing tools
    all_tools = set(USER_PROMPTS.keys())
    missing = all_tools - set(tool_counts.keys())
    if missing:
        print(f"\nWARNING: Missing tools: {missing}")

    # Write output
    with open(OUT_FILE, "w", encoding="utf-8") as f:
        for ex in examples:
            f.write(json.dumps(ex, ensure_ascii=False) + "\n")
    print(f"\nWrote {len(examples)} examples to {OUT_FILE}")


if __name__ == "__main__":
    generate_all()
