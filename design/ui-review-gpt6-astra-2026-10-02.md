# UI/UX overhaul: outside review by GPT‑6 Astra (2026-10-02)

Pranab asked for this review. Astra was given:
- the audit summary and design direction from `ui-overhaul-2026-10-01.md` §1–§6;
- the phase plan;
- one live finding from VM 520: Quick Settings over Files says "WiFi · Disconnected" on a wired, online machine, and the panel is translucent.

Its brief was to critique harshly and give concrete specs. Below is its answer, condensed without dropping any spec. Our response to each point is at the end.

## Its central claim
The reviewer is reacting to a desktop that doesn't reliably explain or control the machine, more than to typography. The 520 finding is the problem in miniature: an online machine claims to be disconnected, offers an ambiguous action, and lets other content bleed through its controls.

"10× better" means: a person can understand the machine, control it, recover from mistakes, and tell what the minds are doing, without studying the shell.

## 1. Where the direction is wrong or naive
- **This is a trust failure, not a styling problem.** These are release blockers:
  - no control drawn without a working backend;
  - no success state before the backend confirms it;
  - no network label derived only from the Wi‑Fi interface;
  - no advertised shortcut without a working binding;
  - no critical action or approval covered by an app.
- **The empty 1,300px of bar is not a diagnosis.** Don't fill it with minds. One chip per mind becomes a new developer status line.
  - A person needs three answers: is anything waiting for me, is anything consequential happening, and where can I look.
  - Progress rings with no real denominator mislead. Show "Working", not an invented 67%.
- **One popover grammar is right; one popover size is wrong.**
  - Share alignment, padding, material, focus and dismissal; don't force everything to 360px.
  - Different dismissal rules by kind:
    - a transient control closes on Escape or a click outside;
    - a form must not silently drop input;
    - closing an approval means it's still pending;
    - lock is never a popover.
- **"Glass-raised" repeats the defect.**
  - On a software renderer, blur is expensive, and translucency without blur just lowers legibility.
  - Start with opaque raised surfaces, a 1px border and a modest shadow.
  - Stillness should extend to rendering too: no constant blur recompute, hidden timers or repainted clocks.
- **The AI vocabulary overlaps.** "Mind mode", "Private (Mind off)", "Mind View" and a mode chip are too much before a person has done anything.
  - **Private is not the same as Off.** Define it first: does it stop capture? Network requests? Running jobs? Logging? Does it keep inference local? What happens to queued work?
  - Prefer explicit words: Pause minds, Local-only processing (only if enforced), Stop screen access, Automatic actions on/off.
  - **"Bypass" is an unexplained developer term for weaker protection.** It must not sit casually beside Dark style. Turning it on should be deliberate, scoped and time-limited, with a persistent warning.
- **Colour can't be the only signal.**
  - Bypass and a failure must differ in icon, label and place, even if both are red.
  - Amber alone can't tell a colour-blind person "waiting" from "paused" from "disconnected".
- **Layer-shell is infrastructure, not the visual answer.** Specify and test each of these:
  - exclusive zones that replace the margins rather than adding to them;
  - which surfaces show over fullscreen apps;
  - which output owns an anchored menu;
  - keyboard focus and dismissal;
  - hotplug and mixed scale;
  - recovery when the panel crashes;
  - whether an app can impersonate or cover an approval.

  Use real popup relationships where possible, and a real session-lock protocol for lock.
- **The order delays basics behind enrichment.**
  - The switcher, working keys, clear privacy state and window navigation come before Today's digest.
  - Clipboard history and recording raise retention, secret-exposure and consent issues. Ship their privacy behaviour with the feature, not later.

## 2. What a first-time user hits in five minutes
Give a new tester these tasks with no coaching:
- open a browser and Files, then go back to Files;
- get online;
- play something quietly;
- make the screen easier to read;
- connect headphones or a display;
- find out what a mind is doing;
- say "no, don't do that";
- find a message that went by;
- leave the machine safely.

Also missing from the stories is a reachable Settings destination for display, keyboard layout, accessibility, audio routing and network details. Quick Settings can't be the only home. Where needed, open an existing reliable tool rather than build another half-working panel. First run should cover readable scale, input, connectivity and what minds may access, not a tour of themes.

## 3. Its top 10, in priority order (logical px)
Test every item at 100%, 150% and 200% scale. Reduced motion means instant state changes. Keyboard focus gets a visible 2px outline. Nothing essential appears only on hover.

1. **Every shell surface is visible, bounded and readable.**
   - Opaque surface, 1px border, 16px radius, 16px padding.
   - Menus sit 8px below their trigger, aligned to its nearest edge, with at least 12px clearance from the screen edge. They flip above, or scroll, when space runs out.
   - Widths: 360px standard, 384px for Quick Settings, 420px for Today, all shrinking to fit.
   - Open with a 120ms opacity change and at most 4px of movement. No scale or bounce.
   - Escape closes and returns focus to the trigger. Opening one panel closes the last; approvals stay pending.
   - Verify all of this over maximised and fullscreen apps on every output, before moving the next surface.
2. **Connectivity and sound as complete interactions.**
   - **Network:**
     - an 18px icon in a 36px target;
     - wired reads "Connected · Wired", and a Wi‑Fi section below says "Wi‑Fi off" or lists networks. Being online over Ethernet never produces a global "Disconnected";
     - rows at least 44px: name, lock, connected state;
     - a click joins or opens the password form; a separate labelled switch is the radio;
     - connecting, authentication failed, captive portal and no internet each read differently;
     - no looping spinner.
   - **Sound:**
     - a 360px panel with a 48px row for the selected output, a slider and mute;
     - a 4px track, a 16px thumb, a 32px hit area;
     - the device's name shown, not just "Volume";
     - no brightness without a backlight, and an unavailable control disabled with a reason.
   - **OSD:**
     - 240×64px, centred, 80px above the taskbar: an icon, a number and a bar;
     - only for the person's own action, gone after 1.2s;
     - it never takes focus.
3. **The bar is built around attention, with a real stop.**
   - Sizes: a 36px top bar and a 48px taskbar.
   - **Left: one mind summary**, 120–200px, not one chip per mind. It reads "Minds ready", "2 minds working", or "1 approval" (amber icon plus text). A click opens a 360px activity panel with 48px rows and an always-visible **Pause all**.
   - **Centre:** `Fri 2 Oct · 14:32`, tabular digits, no seconds.
   - **Right:** capture indicator, connectivity, sound, battery if present, automation mode, Quick Settings.
   - CPU and MEM leave the bar. Bluetooth is hidden when absent or inactive.
   - **Privacy chips:** "Screen shared", "Mic in use", "Camera in use". Name the mind only when attribution is reliable, and offer revocation, not just information.
   - No pulsing; a state change gets one 120ms transition.
4. **Quick Settings is boringly excellent.**
   - Layout: right-aligned under its button, 384px wide, 16px padding, 12px gaps. Two columns of tiles, about 170×64px each: icon, 14px label, optional 12px status.
   - **The toggle and its details chevron are separate hit areas.** Navigating to networks must never look like turning the radio off.
   - Contents: connectivity, Bluetooth when present, DND, appearance. Night light and power profile only once they exist.
   - Below the tiles, volume and brightness sliders using the same component. In the footer: Settings, Lock, Power.
   - Mind controls get one separated section, not three competing tiles. Mind View belongs with workspace navigation if it's a desk.
   - An "on" tile uses a neutral fill plus an explicit on state. Teal stays reserved for minds.
5. **Window navigation comes before Today.**
   - **Taskbar:** 48px high, 32px icons in 40px targets.
     - focused: a filled rounded backing; running: a 3px underline; several windows: a count badge;
     - the title on hover or keyboard focus;
     - a static "Starting…" marker on launch, which clears on success or timeout;
     - a 40px launcher button on the left.
   - **Switcher:** centred, at most 640px wide, 56px rows with a 32px icon, the app name and the title.
     - the selected row gets a background and a focus border;
     - no blank grey thumbnails;
     - Alt+Tab, Alt+Shift+Tab and Escape behave as everywhere else.
   - Show workspace context only when it's useful ("Desk 1 / 3").
6. **Approvals are about consequences, not reassurance.**
   - Retire "Why you can trust this", because it sounds certified. Use **"Details and source"**.
   - Layout: a 420px card anchored to the requesting mind. First line (14px): the mind and a risk label. Then the action (18px). Then up to three consequence rows (14px): what changes, what data leaves the machine and to where, whether it can be undone.
   - **Buttons:** 40px, **Reject** plus an action-specific confirm such as **Send email**.
   - Provenance collapses under Details, at 13px.
   - Sensitive requests show destination and scope without expanding. Dangerous ones open a review surface.
   - No accepting on timeout, no autofocus on the affirmative, and a receipt afterwards. Never imply Undo when only a compensating action exists.
7. **Keys are credible, and Lens routing is explicit.**
   - rc.xml alone isn't the whole truth. Use an **action registry** that generates compositor bindings, shell bindings and help.
   - **Lens layout:** 640px wide, 72px from the top, a 56px input and 48px results, at most six before scrolling.
   - Deterministic matches are labelled **System action**. Mind requests are labelled **Ask mind · may use online processing**. An unmatched command never silently becomes an external AI request.
   - **"wifi off" previews "Turn off Wi‑Fi"; Enter runs it.** Nothing fires while typing.
   - Super+/ shows a searchable sheet of the bindings actually installed. Fix the command palette before calling the Lens the menu.
8. **Notification history before an AI digest.**
   - **Toasts:** 360px, top-right, 12px under the bar, at most three: icon, source, time, two lines, up to two actions. About five seconds, and actionable ones stay in history.
   - **Approvals** never resolve by disappearing.
   - **Today:** 420px, anchored to the clock, with the date and calendar first and notifications under them.
     - empty state: "You're caught up";
     - now-playing (72px) only when there's an MPRIS source;
     - DND hides ordinary toasts but keeps a visible count of waiting approvals.
   - **A digest comes later.** Every claim in it links to a real operation ("Three files updated", not congratulatory prose).
9. **Lock and power are finished features.**
   - **Power menu:** 280px, anchored to its trigger, 44px rows for Lock, Suspend, Log out, Restart and Shut down, plus Hibernate when supported. A compact confirm lists running mind jobs and inhibitors. No full-screen dim.
   - **Lock screen:** a real session lock, a static wallpaper under a dark scrim (no live blur), and a 320px centre block: 64px avatar, name, 44px password field, layout and Caps Lock indicators. The date and time sit above, and battery and network in a corner. Errors stay visible without shaking, notification content and media are hidden by default, and what minds do while the machine is locked is a defined policy.
10. **One excellent theme before seven.**
    - Ship robust dark and light first.
    - Type: 14px body, 12px minimum secondary, 18–20px titles. Contrast holds over every wallpaper.
    - Appearance previews are 160×100px and keyboard-reachable. A theme change applies everywhere together, or says what needs a restart, with no crossfade on the software renderer.
    - **First run is one "Make this comfortable" flow:** scale and text, keyboard layout, appearance, and what minds may access.
    - **Enlarged controls** mean 44px or larger targets.
    - **Stop condition:** add no more themes until the shell works on a high-contrast wallpaper, at larger text, on a small display, with no mouse.

## 4. Minds that beat Omarchy, and how they could lose
- **Better 1: permission-aware delegation with visible scope.**
  - "Prepare the release notes" starts as a bounded job: can read X, can change Y, cannot publish or send.
  - Safe steps inside that scope don't ask; publishing does.
  - The result is less supervision without giving up control.
- **Better 2: a trustworthy work receipt.** "Release notes draft ready · Updated 1 document · Used 6 commits · Nothing published · Open draft · Inspect changes." Reversible transactions where possible, a delivery record otherwise.
- **Better 3: contextual diagnosis from structured state.**
  - "The call is playing through HDMI. Your headphones are connected. **Switch call to headphones**."
  - The mind explains, and a deterministic action makes the change, with no screenshots and no broad screen access.
- **Worse 1: surveillance dressed as companionship.** A permanent roster of minds, a vague "Private" and unexplained mic use make the computer feel occupied. Never claim a mind has stopped while its subprocess still captures or transmits.
- **Worse 2: approval fatigue, then Bypass.** Approve bounded intentions, allow safe steps within their scope, and interrupt only for real changes in destination, data, privilege or reversibility. A standing grant has an expiry and a visible place to revoke it.

**Its blunt recommendation:** freeze Today's AI digest, the extra themes and decorative mind chips until a new user can launch, switch, connect, adjust sound, reject an action and lock the machine without confusion.

---

## Our response (2026-10-02)

**Adopted at once.** These fit decisions already made and need no new ones from Pranab:
- **Opaque raised surfaces.** The kit popover goes opaque with a 1px border; this is the 520 translucency bug. New story 1.1b.
- **Wired never reads "Disconnected".** Already in story 1.3 (s06), now with the 520 evidence.
- **Toggles and their chevrons are separate hit areas,** and no network label comes only from Wi‑Fi. Into stories 1.3 and 2.2.
- **Window navigation moves ahead of Today:** taskbar states and one switcher with icons. New story 3.0, ahead of 3.1.
- **Approval card:** "Details and source", an action-specific confirm, no autofocus on Allow, and consequence rows. New story 0.4. It touches approvals, so it gets a security review.
- **Lens deterministic commands preview, and Enter runs them.** Into story 7.3.
- **No progress rings without a denominator.** Into the bar story.
- **Widths per popover kind,** with one shared grammar. Into story 2.2 and the kit.
- **Per-step checks for live missions.** The Director's page check proves the page runs, not that each step is done. Starfall was accepted with its Sound button unwired. Live follow-up.

**Already true, so no change:**
- The lock is the compositor's session lock (#313).
- Approvals never resolve on timeout; they expire denied.
- Standing permissions with expiry and revocation are saga task 27.

**For Pranab to decide:**
1. Bar heights of 36px and 48px (today 32px and 40px).
2. One mind summary on the bar, instead of one chip per mind.
3. Renaming or defining **Private**, and making **Bypass** deliberate, scoped and time-limited with a persistent warning. That is a product and security choice.
4. Freezing Today's digest and the extra themes until the basic-tasks test passes.
5. Whether to add a first-five-minutes usability test with a new tester as the release gate for the overhaul.
