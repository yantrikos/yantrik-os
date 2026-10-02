# Review of PR #579 — on-screen display (volume, brightness, mic, Caps Lock), branch `ui/osd`

Method: `git diff origin/main...origin/ui/osd` read in full, against `docs/agents/playbook.md`.
Run: `cargo test -p yantrik-os` on the branch: **58 passed, 0 failed** (includes the three new `capslock` tests). `bash -n tests/ui-preview/validate.sh` is OK.
Not run: `yantrik-ui` tests and the Slint build. Findings about `yantrik-ui` come from reading the code.

## Verdict: MERGE AFTER FIXES (1 BLOCKER, 3 SHOULD-FIX)

## Playbook checks
| Check | Result |
|---|---|
| One concern | Mostly. Moving `set_volume`, `set_mute` and `set_brightness` out of `control.rs` into `control_levels_actions.rs` is part of the same concern. |
| Tokens | `osd-bg`, `osd-text` and the other new colours are tokens. They are single hex values for both modes, which is intended. `22px` and `44px` appear as literals in `y_osd.slint`. |
| Timers | One `Timer { interval: 1200ms; running: root.shown; triggered => shown = false }`. It is a readable literal, single-shot in effect, and stops itself. OK. The `dur-osd-hold` token was removed, and the comment says why. |
| Blocking calls | Every actuator runs in `answer_later`. See NIT 3 for the fallback branch. |
| Parity | `describe` keeps its fields. There are 5 actions with grades. See SHOULD-FIX 1. |
| `hold_windows` | Deliberately not called. See BLOCKER 1 and the note after it. |
| True copy | The pill shows the machine's read-back value. The absent-backlight case shows nothing. Good. See SHOULD-FIX 3 for Caps Lock. |

## Findings

### BLOCKER 1 — the pill is drawn in the shell's own window, which sits behind app windows
`crates/yantrik-ui-slint/ui/app.slint` (the new `osd := YOsd`) and `wire/osd.rs::show`. Nothing raises the shell, and no layer-shell surface is used.
Failure scenario: the person is in Firefox, a terminal or a video, all maximised over the shell. They press the volume key. `yos act shell set_volume step=5` changes the volume and the pill is drawn, but in the shell window behind the app. The person sees nothing. That is the main use of an OSD.
Evidence in this repo: `wire/shell_overlays.rs` states that panels drawn by the shell's window are invisible behind an app window unless `windows::raise_shell()` runs (#219, "from Notes the bar's power button worked and showed nothing"). The OSD has no equivalent.
I could not check this on a machine. The PR description should say what was run.
Fix: the OSD needs its own always-on-top surface (a layer-shell overlay or a small separate window). Calling `raise_shell()` on every volume key would take focus away from the person's app, so it is not a fix. If the shell window is already top-layer somewhere I missed, show the evidence in the PR and I will withdraw this.

Note on `hold_windows`: the PR's argument that the pill is inert, so `hold_windows` is not needed, is sound for the pill. It stops holding once a separate surface is introduced. A new surface that is raised above a waiting approval card needs the same analysis as the other overlays, and a mind can trigger it by calling `set_volume`.

### SHOULD-FIX 1 — unmuting the microphone is graded `standard`
`control_levels_actions.rs` `set_mic_mute`: `.risk("standard")`, with `muted: false` and `toggle`.
Failure scenario: a mind calls `set_mic_mute muted=false`, or `toggle=true` on a muted mic, and the microphone is live. Muting is harmless, but unmuting is a privacy act: the machine can hear the room. The grades exist so a person controls such acts.
Fix: grade the action `sensitive`, or split it (`mute_mic` standard, `unmute_mic` sensitive). A toggle that resolves to unmute should also be graded as an unmute.

### SHOULD-FIX 2 — a key step is now read-then-write instead of one atomic `wpctl 5%+`
`control_levels.rs::set_volume` (and `set_brightness`): it reads the current level, computes the target, then `set_volume(target)`.
Failure scenario: a held volume key repeats about 30 times per second, and each press is a separate `yos` process and a separate worker. Two presses read the same level and write the same target, so steps are lost and the volume stutters. The old binding `wpctl set-volume … 5%+` had no race.
The same applies to `step` from a mind. Also, the old binding capped at 1.0 with `-l 1.0`, but now there is no explicit `-l`.
Fix: for a step, call `wpctl set-volume @DEFAULT_AUDIO_SINK@ N%+/-` (and `brightnessctl set N%+/-`) and read back afterwards. At minimum serialise the actions with a mutex.

### SHOULD-FIX 3 — the Caps Lock pill reads the LED right on key release and may show the old state
`rc.xml` `Caps_Lock onRelease="yes"` → `show_caps_lock` → `capslock::read()` (sysfs `::capslock` LED).
Failure scenario: the LED brightness in sysfs is updated by the kernel after the compositor sends the LED event, and the `yos` round trip may be faster than that. The pill then says "Caps Lock off" when it is on. This is the opposite of "true copy". The PR comment asserts the LED "is what the shell reads" by release, without a measurement. On a machine with no `::capslock` LED, nothing is shown, which is correct.
Also unverified: that binding the key (even on release) does not stop the toggle reaching the client in labwc.
Fix: read the xkb state the compositor holds, or delay the read about 50 ms and re-read; and test the key on a real machine.

### NIT 1 — fallback `||` still fires after a refusal
`rc.xml`: `yos act … || wpctl …`. When the shell refuses (a locked screen, or a `hold_windows`-style refusal), the direct command runs, with no pill. That is documented in the rc.xml comment and is reasonable. It also means the control-surface grade is bypassed by the key. That is fine for physical keys. Say so in the PR text.

### NIT 2 — literals
`y_osd.slint`: `size: 22px`, `min-width: 44px`. Tokenise.

### NIT 3 — `off_the_ui_thread` falls back to running the work inline
`control_levels_actions.rs`: `answer_later(work).or_else(|work| work())`. When no dispatch slot is open, `wpctl` runs on the calling thread. That is only the no-socket path, but then it is exactly the blocking call the playbook forbids. Prefer an error.

### NIT 4 — external processes have no timeout
`audio.rs::read_mic_muted` and `run_wpctl` use `Command::output()` without a timeout. A hung `wpctl` pins a worker. Add the usual ~2 s kill.

## Verified OK
- The media-key actions are the same ones Quick Settings uses, and answers read back the machine's value.
- A mind's volume change goes through the same actions, so it shows the pill (once BLOCKER 1 is fixed).
- The echo from the audio watcher does not show the pill.
- A step up from a muted speaker unmutes. This is documented and tested.
- `show_caps_lock` is `safe` and changes nothing.
- `rc_keys.rs` has a new test that every media key goes through `yos act shell`.
