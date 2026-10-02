# Security review: PR #573 (`ui/network-indicator`, head f0aa41b)

Reviewed `git diff origin/main...origin/ui/network-indicator` (28 files, +3528/-489). Read in full:
`yantrik-os/src/network.rs`, `network_model.rs`, `yantrik-ui/src/control_network.rs`,
`wire/network.rs`, the Slint popover/state/indicator/status-bar/quick-settings/overlay changes,
and the touched parts of `control.rs`, `callbacks.rs`. Also read the surrounding runtime
(`yantrik-app-runtime/src/control.rs` `regrade`/`answer_later`, `yantrik-surface` registry),
`control_overlays.rs`, `wire/shell_overlays.rs`, and the pre-existing mind tools in
`yantrik-companion-tools/src/wifi.rs` and `apps/network-manager`.

## Overall verdict: MERGE AFTER FIXES

The headline design (a mind cannot supply a password, cannot open or focus a field, cannot
create a profile through the control surface) is sound, and I could find no path by which a mind's
action moves focus or alters typed text. The remaining problems are in the grade story (stale
grade, and a downgrade below what the repo already assigns to the same verbs), one missing
defence-in-depth check in `connect()`, and unbounded per-call resources reachable by a mind.
None is exploitable to steal a password.

## What was run

* `cargo test -p yantrik-os --lib network` : **21 passed, 0 failed** (network.rs and
  network_model.rs tests, including the timeout/sender/bounded-wake source scan, the
  autoconnect-only-when-by_person test and the secret zeroize/Debug tests).
* `cargo test -j2 -p yantrik-ui --bin yantrik-ui control_network` : build failed (missing speechd system library); not run. See the last section.
* Read-only static review of everything else. No VM, no real NetworkManager, no D-Bus, so
  none of the runtime behaviour (match-rule filtering, timeouts, join flow) was exercised; those
  verdicts rest on code reading and the existing source-scan tests.

---

## Findings

### HIGH

**H1. `disconnect_network` / `set_wifi` are published `standard` where the repo already grades the same verbs `dangerous`.**
`crates/yantrik-ui/src/control_network.rs:63-72` (`grade_for`), `:77-82` (`sync_grades`).
The mind tool registry (`crates/yantrik-companion-tools/src/wifi.rs:313,352`) grades
`wifi_disconnect` and `wifi_radio` **Dangerous**, and `apps/network-manager/src/main.rs:1131,1166`
declares the same actions `dangerous`, with a long comment that "the same verb should not be cheaper
because the mind asked for it" and that what is destroyed is "the channel" a remote mind uses.
This PR declares them `sensitive` and publishes `standard` whenever `links >= 2`. A mind that
is refused or carded on `wifi_disconnect` can call the shell's `disconnect_network` /
`set_wifi enabled=false` at a lower grade. "Another link remains" does not mean the mind's own
path remains: a mind reaching the box over the Wi-Fi link, with a wired link also up, is cut off
at `standard`. `links` also counts any up Wi-Fi/Ethernet profile, including one with
`connectivity=none`.
Fix: declare both `dangerous` (matching the app and the tools) and publish `sensitive` at the
lowest, never `standard`; or, if a lower grade is intended, state that as a decision and align the
two other surfaces. Do not count a link as "another connection" unless it has connectivity.

**H2. Grade is read from a stale snapshot, and nothing re-checks at execution (TOCTOU).**
`control_network.rs:196-211` (`set_wifi`), `:223-235` (`disconnect_network`);
`wire/network.rs:314-315`; `yantrik-os/src/network.rs:179-181,508-529`.
The gate asks the grade published at dispatch time, which is as of the last *published* reading:
NetworkManager signal, then up to 2 s of settle (`SETTLE_CAP`), then a multi-round-trip re-read, then
a hop to the UI thread. The handler then defers its D-Bus work to `answer_later`, which runs later
on the RPC side. Neither the handler nor the worker recomputes `grade_for` from fresh state, and
`disconnect()` picks its own target from a fresh D-Bus read, not the snapshot the grade came from.
Scenario: wired + Wi-Fi up (links=2, published `standard`); the cable drops (or the mind's
earlier call took one down); within the lag window the mind calls `disconnect_network`
or `set_wifi enabled=false`; it runs at `standard` and removes the last link, with no card.
The same lag also applies in the other direction (publishing `sensitive` late is harmless).
Fix: in the handler (UI thread, after the gate) compare `published_grade(action)` with
`grade_for(action, latest())` and refuse with "state changed, retry" if the fresh grade is higher.
In the worker, re-read via `read_snapshot` on the connection it already has and re-run the
same check immediately before the `Disconnect`/`Set` call, so the decision and the act use one reading.

### MEDIUM

**M1. `yantrik_os::network::connect()` does not itself refuse a mind-initiated create.**
`crates/yantrik-os/src/network.rs:581-603,686-688`. The "a mind may only switch to a saved network"
rule is enforced only in `control_network::plan_connect`, on `latest()`, which can be seconds stale,
and `connect()` then re-plans on a fresh read and will happily run `Plan::JoinOpen` (create profile +
join) for a `by_person: false` request; `by_person` only turns autoconnect off. Scenario: a
saved open profile is removed (the person deletes it in Settings, or a failed person-join
cleanup deletes it) between the handler's snapshot (`known=true` so `Join`) and the worker's fresh
read (`known=false`, `!secured`, so `JoinOpen`); the mind has now silently joined an open network that
was never saved. Item 2's "the person must press Join" is therefore a property of one caller, not of the API.
Fix: in `connect()`, `if !request.by_person && !matches!(decided, Plan::UseSaved) { return Err(...) }`
(before any D-Bus write). Keep the autoconnect=false line as a second layer; consider always
setting autoconnect=false for open networks (see L2).

**M2. A mind can start unbounded threads, D-Bus connections and joins.**
`wire/network.rs:205-241` (`start_join`: one OS thread per call, up to 45 s each, no in-flight
guard); `yantrik-os/src/network.rs:462-468,473,490,508,558` (`system_bus()` builds a new zbus
connection, with its executor thread and socket, on *every* call); `control_network.rs:210,234`
(`answer_later` work runs under `block_in_place` per call). `connect_wifi` on any saved
network is `sensitive` and, in Auto mode, runs without a card, so a mind can loop it (or alternate
two saved networks) to pile up joiners that fight each other (`ActivateConnection` flip-flops),
hold sockets against the system bus's per-uid connection limit, and starve the shell's own
D-Bus use. Item 5's "unbounded channel/thread" fix covers only the signal wake channel.
Fix: one lazily created shared `Connection` (with the timeout); a single in-flight join guard
(reject or coalesce while one is running); a serialising worker or a `Mutex` around the three
mutating operations; rate-limit `connect_wifi`.

**M3. A mind can disconnect a wired link that the popover cannot bring back.**
`yantrik-os/src/network.rs:503-529`; popover `network_popover.slint:196-221`.
`Device.Disconnect` blocks that device's autoconnect until asked. For Wi-Fi the popover can rejoin;
for a wired device the popover has no connect control, so the machine stays unplugged until a reboot or
a manual `nmcli`, and `connect_wifi` is no help. Combined with H1/H2 this is a one-call availability hit.
Fix: after a mind-initiated disconnect schedule/offer a re-enable (e.g. `Reapply`/`ActivateConnection`
for wired), or refuse wired targets from the control surface.

**M4. The signal listener's exit silently kills the live network picture.**
`yantrik-os/src/network.rs:154-157,176-178`. On any `Err` from the match-rule iterator the
listener returns, dropping `wake_tx`; the monitor then gets `recv() == Err` and returns, so the bar
and `describe` freeze at the last reading (grades included) with no retry and no log line. Not a spoofing
issue (`.sender(NM)` is correct), but a frozen snapshot is what the grade logic trusts (H2).
Fix: log and resubscribe/retry with backoff; on exit, publish a "stale" reading or fall back to polling.

### LOW

**L1. `describe shell` discloses nearby and saved networks to every mind.** `control_network.rs:122-140`:
all visible SSIDs with signal and `known` (which networks this person has joined before). That is
location- and history-revealing, and is exposed to untrusted minds at read grade. Consider limiting
`known` and SSID lists to what `connect_wifi` needs, or grading the read.

**L2. The "asked to join" row gives a mind a one-click path to an open evil twin that autoconnects.**
`network_popover.slint:113-120`, `network.rs:686`. A mind can mark any visible open SSID; the row
turns warning-coloured with a prominent Join button and a persistent attention dot. A person's
Join is `by_person: true`, so the new profile gets NetworkManager's default autoconnect=on, which
makes an impersonated open network permanent. Suggest autoconnect=false for open networks always,
and a one-line "unsaved open network: traffic is not encrypted" caption on that row. The mark also has no
expiry (cleared only on join success or popover close).

**L3. Attribution on the mark is the active mind, not the caller.** `control_network.rs:149-153`:
`get_active_harness_name()` is what the person selected, not who made the call, so "<mind> asked to
join X" can be false when another caller or process drove the socket. Use the caller identity
(`caller()` / agent token) already available to handlers.

**L4. `answer_later(...).or_else(|work| work())` runs D-Bus inline on the caller's thread when the
handler is invoked outside a dispatch** (`control_network.rs:210,234`). Only reachable from in-process
or test calls today, and the source-scan test (`no_handler_calls_dbus_on_the_ui_thread`) cannot see it.
Fix: return an error there, not run the work.

**L5. A join poll that hits the 2 s call timeout is treated as failure.** `network.rs:635-651`: `get_all`
returning `None` falls into the `_` arm, which deletes the just-created profile and deactivates a
connection that may have been succeeding. Distinguish "no answer" from "state is failed" and keep polling to the cap.

**L6. Secret-hygiene claims are stronger than the code.** `WifiSecret` zeroizes its `String`, but the
Slint `SharedString` copy (`wire/network.rs:164-169`), the field's own value, and zbus's serialized
message buffer (`network.rs:610-617`) are not zeroed; the comments ("overwritten", "held in exactly one place")
overstate this. Not remotely exploitable; reword, and avoid `secret.to_string()` copies where possible.
Checked and fine: zbus's `Debug` for `Message` prints only the body *signature*, so `RUST_LOG=zbus=trace`
does not log the PSK; no `nmcli`/`Command` in the new files; no log line names the request; error
text from NM does not echo the settings; `ConnectRequest`/`WifiSecret` print `<given>`/`<secret>`.

**L7. Panel stacking via `open_quick_settings`.** `control_overlays.rs:97` closes only its own three
panels, so opening Quick Settings from the control surface while the network popover is open leaves
both up. (`hold_windows` is called there as required.) Cosmetic, but cheap to add `network_open` to
`Panel::ALL`'s peers.

**L8. Saved-profile lookup is by lossy name.** `network.rs:432,570,666` compare a
`from_utf8_lossy` string against raw SSID bytes, so non-UTF-8 SSIDs show `known` but fail to
activate ("no saved profile"). Fail-safe, but wrong.

**L9. Related, pre-existing, not in this diff but undermines the model:** the companion tool
`wifi_connect` (`yantrik-companion-tools/src/wifi.rs:247`, `Sensitive`) takes a password from a mind and
can create a profile for an unknown network. The comment block in `control_network.rs` ("no password
crosses this surface") is true only of the shell surface; the person-must-type guarantee does not hold
machine-wide while that tool is registered. Track separately.

### Checked, no finding

* **Mind action changing focus, keyboard routing or typed text:** none found.
  `mark_for_person` only sets `requested_ssid`/`requested_by`; it does not touch `asking_ssid`,
  the popover's `open` state, focus, or `raise_shell`; its source-scan test enforces this. The only
  `self.focus()` is the password field's `init`, which runs only when `asking_ssid` is set, which is
  set only by the Slint row's own `activated` (a person's click/Enter/Space). `grep` shows no
  `set_asking_ssid`/`set_network_open` in any `control*.rs`. Residual: if the asked row leaves range and
  returns while `asking_ssid` is still set, the field is recreated and re-focuses (popover must be open).
* **Internal callbacks / `by_person`:** `NetworkState.connect/disconnect/set-radio` are Slint
  callbacks reachable only from the Slint tree; `by_person: true` is hard-coded in `on_connect`.
  I found no control-surface action that synthesises pointer/keyboard events or invokes a callback by name.
* **Windows/overlays without `hold_windows`:** the network popover cannot be opened from the control
  surface at all (`Panel::ALL` is unchanged, no network panel action). It opens only from the bar click;
  `changed network-open` calls the existing `raise_shell`, same as the other bar panels.
* **Signal spoofing:** `.sender(NM)` is a well-known-name match, which the bus daemon resolves to the
  current owner; `path_namespace` is also set. A non-NM sender cannot wake the monitor. Even if it
  could, the wake only triggers a re-read from NM.
* **Passwords in `describe`:** none; test asserts neither `asking` nor `NetworkState` is read there.

---

## Verdicts on the 7 items from the previous review

1. **FIXED.** `connect_wifi` (`control_network.rs:254-275`) calls `start_join` only for `ConnectPlan::Join`
   (profile already saved, `secret: None`, `by_person: false`) and otherwise `mark_for_person`, which sets
   two properties and returns. No path from the control surface sets `asking_ssid`, opens the popover, focuses
   or raises (`marking_a_row_never_opens_replaces_or_focuses_a_password_field` scans for it). The Slint row
   renders "asked to join" and the bar gets `attention`. Caveats in L2/L3, not a focus issue.
2. **PARTIAL.** Control surface: an unsaved open or secured network is never joined (`plan_connect`), and the
   `build_settings` path sets `autoconnect=false` for non-person open joins. But the guarantee is not
   enforced in `connect()` itself and the handler decides from a stale snapshot, so a snapshot
   race can still create-and-join an open network silently (M1). Also see L2 for the person-side autoconnect.
3. **PARTIAL.** Declared `sensitive`, `regrade` on every reading and on `describe`, `sensitive` until the
   first reading; verified in code and tests. But the published grade lags the machine and is not
   re-checked at execution (H2), and the "standard" level itself is below the repo's `dangerous` for the same
   verbs (H1).
4. **FIXED.** `set_wifi`/`disconnect_network` do their D-Bus work in `answer_later`; `connect_wifi`'s handler
   makes no D-Bus call (`start_join` spawns); `new_connection()` sets `method_timeout(2 s)`; scan/radio/
   disconnect callbacks spawn threads; source-scan test present and passing. Residual: L4 (inline fallback) and M2.
5. **FIXED** (with M2/M4 caveats). `.sender(NM)` present; settle loop bounded by `SETTLE_CAP` = 2 s; wake
   channel is `bounded(1)`; test enforces all three. The unbounded growth that remains is per-call, not the signal path.
6. **FIXED.** The Quick Settings Wi-Fi tile now calls `NetworkState.set-radio` (the radio, via D-Bus, with a
   truthful caption), the old `nmcli`-based `toggle-wifi` is gone from `callbacks.rs`/`app.slint`/overlays, the
   tile is hidden with no Wi-Fi device, and a visible Quick Settings button exists at the far right of the bar.
7. **FIXED.** One thread-local `ROWS: Rc<VecModel>` updated in place by `apply_rows` (keyed by SSID; order
   frozen while the popover is open or a field is asking), so a rescan no longer rebuilds the row element or
   its half-typed text. Edge in the "Checked, no finding" section.

---

## Suggested fix order
1. H1 and H2 together: fix the grade (dangerous / never `standard`) and recheck on fresh state at the
   handler and again in the worker.
2. M1: refuse `!by_person && !UseSaved` inside `connect()`.
3. M2: shared connection + single in-flight join + rate limit.
4. M3/M4, then the LOWs.

## Shell crate run

`cargo test -j2 -p yantrik-ui --bin yantrik-ui control_network` did **not** build: the dependency
`speech-dispatcher-sys v0.7.0` build script failed (system library `speechd` / pkg-config missing; I did
not apt-install it, to stay within the sandbox budget). So none of the yantrik-ui tests
(`control_network`, `wire::network`, `control_approvals`, `card_watch`) were run; their assertions
were read, not executed. The preview crate (`tests/ui-preview`) was not run either.
