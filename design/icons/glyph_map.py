"""Which Phosphor glyph (fill weight, vendored in glyphs/) each app wears on its tile.

Keyed by the shell's app ids, the same ones `AppColor.hue-for-app` and `Icons.app` use, so an app
has one identity. The generator refuses to run while an app in the colour table has no line here.

Where the monochrome glyph (`Icons.app`) already made a choice, this follows it, so a tile and a
menu row show the same object: Studio is a spark, not a second picture, because `image` is
already the picture; Agents is people, one per agent working.
"""

GLYPHS = {
    # Writing
    "notes": "notepad",
    "editor": "code",
    "documents": "file-text",
    "spreadsheet": "table",
    "presentation": "presentation-chart",
    "snippets": "scissors",
    # Communication
    "email": "envelope-simple",
    "calendar": "calendar-dots",
    "notifications": "bell-simple",
    # Media
    "music": "music-notes",
    "media": "film-strip",
    "image": "image",
    "studio": "sparkle",
    "blender": "cube",
    "arcade": "game-controller",
    # The files, and the machine
    "files": "folder-simple",
    "terminal": "terminal-window",
    "system": "cpu",
    "sysmonitor": "gauge",
    # htop's own icon is a photographic bar chart; this is the flat one.
    "htop": "chart-bar",
    # Not wifi-high: filled, its arcs merge into a wedge that reads as a pie slice.
    "network": "network",
    "containers": "stack",
    "packages": "package",
    "devices": "devices",
    "downloads": "download-simple",
    "browser": "globe",
    "weather": "cloud-sun",
    # The companion
    "memory": "brain",
    "agents": "users-three",
    "recipes": "list-checks",
    "bond": "heart",
    "personality": "smiley",
    "skills": "lightning",
    "permissions": "shield-check",
    # Ours, not Phosphor's (glyphs/README.md): every Phosphor gear has round, shallow teeth that
    # merge into a flower at 32px.
    "settings": "settings-gear",
    "about": "info",
    "launchpad": "squares-four",
}
