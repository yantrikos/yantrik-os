//! The stack's model: where each window sits, how long ago it was used, and what the caption
//! says. No Wayland and no Slint in here, so every rule is a test.
//!
//! labwc owns the order and the selection (the switcher session, `zlabwc_window_switcher_
//! renderer_v1`); this only draws what it is told. A card's depth is how far it is behind the
//! selected window in labwc's own order, so Tab, which moves labwc's selection, moves the stack.

/// One window of a session, as labwc sent it. `app_id` and `title` come from the window's own
/// client: untrusted text, cleaned by [`clean`] before anything draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: u32,
    pub app_id: String,
    pub title: String,
    pub workspace: String,
    pub current_workspace: bool,
    pub minimized: bool,
    pub focused_at_start: bool,
    /// Milliseconds since the window last had focus; 0 when labwc never saw it focused.
    pub idle_ms: u32,
}

/// How many cards are drawn: the selected one and four behind it. The rest are counted.
pub const VISIBLE: usize = 5;

/// The slot a card sits in, in the 1280×800 frame the design was drawn in; the caller scales
/// it to the output. A card is its window's picture, fitted into the slot (`fit`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub depth: usize,
}

/// The slot at depth `d` (GPT-6 Astra's art direction, 20 Oct 2026): each step back is a tenth
/// smaller and moves up and right by less than the last (144, 128, 112, 96 across; 64, 56, 48,
/// 40 up), so every card behind the front one shows a band of its top and a strip of its right
/// side, its own window content rather than a stack of title bars:
///
/// | depth | x   | y   | w   | h   |
/// |-------|-----|-----|-----|-----|
/// | 0     | 80  | 240 | 640 | 400 |
/// | 1     | 224 | 176 | 576 | 360 |
/// | 2     | 352 | 120 | 512 | 320 |
/// | 3     | 464 | 72  | 448 | 280 |
/// | 4     | 560 | 32  | 384 | 240 |
///
/// `d` may be fractional (mid-glide), past the last drawn depth (fading out at the back) or
/// below 0: the card that was in front, leaving past the viewer, grows a tenth by -1 and drifts
/// a little left and up, which reads as coming toward you without any perspective.
pub fn place_at(d: f32) -> Place {
    let (x, y) = if d < 0.0 {
        (80.0 + 40.0 * d, 240.0 + 14.0 * d)
    } else {
        (80.0 + 144.0 * d - 8.0 * d * (d - 1.0), 240.0 - 64.0 * d + 4.0 * d * (d - 1.0))
    };
    let k = (1.0 - 0.1 * d).max(0.3);
    Place { x, y, w: 640.0 * k, h: 400.0 * k, depth: d.max(0.0).round() as usize }
}

/// A picture of aspect `aspect` (width / height) in the slot (x, y, w, h): as large as fits, its
/// window's own shape, with no frame round it. The front card sits in the slot's bottom-left
/// corner, so its left edge and the caption under it line up whatever the window's shape; a card
/// behind sits top-right, keeping the diagonal's outer edge. Mid-glide it slides between the two.
pub fn fit(slot: (f32, f32, f32, f32), aspect: f32, d: f32) -> (f32, f32, f32, f32) {
    let (x, y, sw, sh) = slot;
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.6 };
    let w = sw.min(sh * aspect);
    let h = w / aspect;
    let back = d.clamp(0.0, 1.0);
    (x + (sw - w) * back, y + (sh - h) * (1.0 - back), w, h)
}

/// How much of the ground's navy is laid over a card at depth `d`: 0 in front, then 8, 15, 22
/// and 28%. The stack stays solid; deeper is darker, never see-through (a translucent window
/// looks spectral, not far away).
pub fn dim(d: f32) -> f32 {
    let table = [0.0, 0.08, 0.15, 0.22, 0.28];
    let d = d.clamp(0.0, 4.0);
    let i = (d.floor() as usize).min(3);
    table[i] + (table[i + 1] - table[i]) * (d - i as f32)
}

/// A card's opacity: 1 in the stack. The card leaving past the front stays whole for its first
/// stretch (to -0.3) and is gone by -0.9; one leaving past the last drawn depth fades from 4 to 5.
pub fn fade(d: f32) -> f32 {
    if d < 0.0 {
        ((d + 0.9) / 0.6).clamp(0.0, 1.0)
    } else if d > 4.0 {
        (1.0 - (d - 4.0)).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// How much a card shows as the selected one, 0..=1: its edge turns from the stack's grey to
/// the accent over the last third of its way to the front, and stays so as it leaves.
pub fn chosen(d: f32) -> f32 {
    if d < 0.0 {
        1.0
    } else {
        (1.0 - d * 3.0).clamp(0.0, 1.0)
    }
}

/// The parts of a card (x, y, w, h) that the opaque card in front of it, `cover`, leaves showing,
/// in the card's own coordinates: at most four rectangles round the covered part, or the whole
/// card when nothing covers it. Only these are drawn. On this software renderer the pictures and
/// the depth veil hidden under the front card were half of every full frame (520, 20 Oct: 19-21
/// ms a frame with them, 10-11 without). The cover is taken `inset` smaller each side, so the
/// pieces run on under its rounded corners rather than stopping at its square edge.
pub fn uncovered(card: (f32, f32, f32, f32), cover: Option<(f32, f32, f32, f32)>, inset: f32) -> Vec<(f32, f32, f32, f32)> {
    let (cx, cy, cw, ch) = card;
    let whole = vec![(0.0, 0.0, cw, ch)];
    let Some((nx, ny, nw, nh)) = cover else { return whole };
    let (l, t) = ((nx + inset).max(cx), (ny + inset).max(cy));
    let (r, b) = ((nx + nw - inset).min(cx + cw), (ny + nh - inset).min(cy + ch));
    if r <= l || b <= t {
        return whole;
    }
    // Card-local, and whole pixels outward into the covered part, so no seam opens between pieces.
    let (l, t, r, b) = ((l - cx).ceil(), (t - cy).ceil(), (r - cx).floor(), (b - cy).floor());
    let mut out = Vec::new();
    let mut piece = |x: f32, y: f32, w: f32, h: f32| {
        if w > 0.0 && h > 0.0 {
            out.push((x, y, w, h));
        }
    };
    piece(0.0, 0.0, cw, t);
    piece(0.0, b, cw, ch - b);
    piece(0.0, t, l, b - t);
    piece(r, t, cw - r, b - t);
    out
}

/// A value easing to a new one over `dur` seconds (cubic ease-out), retargeted from wherever it
/// is: the time line's selected marker, which glides between windows' times and never jumps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    t: f32,
    dur: f32,
}

impl Tween {
    pub fn at(v: f32, dur: f32) -> Tween {
        Tween { from: v, to: v, t: dur, dur }
    }

    pub fn value(&self) -> f32 {
        let u = (self.t / self.dur).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - u).powi(3);
        self.from + (self.to - self.from) * e
    }

    pub fn retarget(&mut self, to: f32) {
        if to != self.to {
            self.from = self.value();
            self.to = to;
            self.t = 0.0;
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.t = (self.t + dt).min(self.dur);
    }

    pub fn done(&self) -> bool {
        self.t >= self.dur
    }
}

/// How far a card may trail its place, in depths.
pub const LAG_MOST: f32 = 1.0;

/// A card in motion: a critically damped spring from where it is drawn (`pos`) to where it
/// belongs (`target`), both depths in the stack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub pos: f32,
    pub vel: f32,
    pub target: f32,
    /// 0..=1: how far a card that re-entered at the back has faded in.
    pub appear: f32,
    /// Leaving past the front; when it is out of sight it re-enters at the back, at `back_to`.
    pub leaving: Option<f32>,
}

impl Spring {
    pub fn at(d: f32) -> Spring {
        Spring { pos: d, vel: 0.0, target: d, appear: 1.0, leaving: None }
    }

    /// A new place. A card sent from the front to the back of an `n`-window list goes forward
    /// out of sight first (toward -1) rather than back through the whole stack; one brought from
    /// the back to the front starts just in front (-1) and comes in.
    pub fn retarget(&mut self, target: f32, n: usize) {
        let half = n as f32 / 2.0;
        if target - self.pos > half {
            // Aimed past -1, so it is out of sight (-0.95) in about half the time a spring would
            // take to creep up to -1 itself.
            self.target = (self.pos.floor() - 1.6).min(-1.6);
            self.leaving = Some(target);
        } else if self.pos - target > half {
            self.pos -= n as f32;
            self.target = target;
            self.leaving = None;
        } else {
            self.target = target;
            self.leaving = None;
        }
    }

    /// A new place reached directly, never round the end of the list (a jump).
    pub fn glide_to(&mut self, target: f32) {
        self.target = target;
        self.leaving = None;
    }

    /// Advance by `dt` seconds (semi-implicit Euler in small steps, which is stable for this
    /// stiffness at any frame rate).
    pub fn step(&mut self, dt: f32, stiffness: f32, appear: f32, n: usize) {
        let mut left = dt;
        while left > 0.0 {
            let h = left.min(0.004);
            let a = -2.0 * stiffness * self.vel - stiffness * stiffness * (self.pos - self.target);
            self.vel += a * h;
            self.pos += self.vel * h;
            left -= h;
        }
        // Never more than LAG_MOST behind its place: a held Tab moves places faster than a soft
        // spring follows, and the card in front must stay the one Alt's release would choose.
        if self.leaving.is_none() && (self.pos - self.target).abs() > LAG_MOST {
            self.pos = self.target + LAG_MOST * (self.pos - self.target).signum();
        }
        if self.appear < 1.0 {
            self.appear = (self.appear + dt / appear).min(1.0);
        }
        if let Some(back) = self.leaving {
            if self.pos <= -0.95 {
                // Out of sight in front: in again at the back, fading in, from just behind its place.
                self.pos = back + 0.6;
                self.vel = 0.0;
                self.target = back;
                self.appear = 0.0;
                self.leaving = None;
            }
        }
        let _ = n;
    }

    /// Close enough to stop: within about a pixel of its place (a depth is ~100 px) and all
    /// but still. The spring's tail would take another 0.3 s for that last pixel.
    pub fn settled(&self) -> bool {
        self.leaving.is_none() && (self.pos - self.target).abs() < 0.01 && self.vel.abs() < 0.1 && self.appear >= 1.0
    }

    /// Moving fast enough that frames may be drawn at reduced size without it showing: still
    /// far from its place, quick, or leaving past the front.
    pub fn fast(&self) -> bool {
        self.leaving.is_some() || self.vel.abs() > 2.0 || (self.pos - self.target).abs() > 0.25
    }

    pub fn snap(&mut self) {
        self.pos = self.target;
        self.vel = 0.0;
        self.appear = 1.0;
    }
}

/// How far `index` is behind `selected` in an `n`-window list.
pub fn depth(index: usize, selected: usize, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (index + n - selected % n) % n
}

/// Unicode's format characters (general category Cf): bidi overrides and isolates, zero-width
/// spaces and joiners, the soft hyphen, word joiners, the BOM, interlinear annotation marks and
/// tag characters. None draws anything a person can see, and each can make one title look like
/// another or turn text around, so a title loses them all.
pub fn is_format(c: char) -> bool {
    matches!(c,
        '\u{00ad}' | '\u{0600}'..='\u{0605}' | '\u{061c}' | '\u{06dd}' | '\u{070f}' | '\u{0890}'..='\u{0891}'
        | '\u{08e2}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}'
        | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}' | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}'
        | '\u{e0020}'..='\u{e007f}')
}

/// A title or app id fit to draw: control and format characters (bidi, zero-width) dropped,
/// whitespace runs made one space, and at most `max` characters, ending in "…" when cut.
pub fn clean(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_control() || is_format(c) {
            if c.is_whitespace() || c == '\n' || c == '\t' {
                space = !out.is_empty();
            }
            continue;
        }
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    if out.chars().count() > max {
        let cut: String = out.chars().take(max.saturating_sub(1)).collect();
        return format!("{}…", cut.trim_end());
    }
    out
}

/// What the window is called on its card: its title, or its app id when it has none.
pub fn name(w: &Window) -> String {
    let title = clean(&w.title, 120);
    if title.is_empty() {
        let app = clean(&w.app_id, 60);
        if app.is_empty() {
            "Untitled window".into()
        } else {
            app
        }
    } else {
        title
    }
}

/// One of our apps, from its window's title: exactly its name ("Notes"), or its name with what it
/// has open ("Notes: Handover", "plan.md - Editor"). Our windows set no app_id (#877), so the
/// title is all there is; the shell's own window list matches exactly (it must not count a note
/// as a second Notes), but for an icon the app is what matters.
pub fn our_app(title: &str) -> Option<&'static str> {
    yantrik_ui_kit::app_names::app_id_for_title(title).or_else(|| {
        yantrik_ui_kit::app_names::APP_NAMES
            .iter()
            .find(|(_, name)| {
                title.starts_with(&format!("{name}: "))
                    || title.starts_with(&format!("{name} - "))
                    || title.ends_with(&format!(" - {name}"))
                    || title.ends_with(&format!(" \u{2014} {name}"))
            })
            .map(|(id, _)| *id)
    })
}

/// The id a window's icon is drawn from (AppTile): the shell's icon id for its app id, or for
/// one of ours, which sends none, the app its title names. The Browser this OS ships is Chromium,
/// so its window is drawn with the Browser's icon. Anything else gets its title behind a `~`, which
/// no icon id has, so AppTile draws its initial on the neutral plate: a window titled `files` does
/// not get to wear the Files icon (security review of #876, finding 2).
pub fn icon_id(w: &Window) -> String {
    let declared = clean(&w.app_id, 60).to_lowercase();
    if declared.is_empty() {
        let title = clean(&w.title, 120);
        return our_app(&title).map(str::to_string).unwrap_or_else(|| format!("~{title}"));
    }
    match declared.as_str() {
        "chromium" | "chromium-browser" => "browser".into(),
        _ => yantrik_ui_kit::app_names::icon_id_for(&declared),
    }
}

/// How long ago, said plainly: "now", "4 min ago", "1 h ago", "2 h 15 min ago". `None` when
/// labwc never saw the window focused.
pub fn ago(idle_ms: u32) -> Option<String> {
    if idle_ms == 0 {
        return None;
    }
    let min = idle_ms / 60_000;
    Some(match min {
        0 => "now".into(),
        1..=59 => format!("{min} min ago"),
        _ if min.is_multiple_of(60) => format!("{} h ago", min / 60),
        _ => format!("{} h {} min ago", min / 60, min % 60),
    })
}

/// The time line runs from now (top) to two hours ago (bottom) on a log scale, so the last few
/// minutes, where most switching happens, get most of the room. Older than two hours sits in the
/// "earlier" band at the very bottom. Returns 0.0 (now) to 1.0 (earlier).
pub fn timeline_pos(idle_ms: u32) -> f32 {
    let min = idle_ms as f32 / 60_000.0;
    if min > 120.0 {
        return 1.0;
    }
    0.92 * (1.0 + min).log10() / 121f32.log10()
}

/// Nudge positions apart so no two labels on the time line are closer than `gap`: each keeps its
/// order and moves down only as far as it must. Returned in the input's order.
pub fn spread(ys: Vec<f32>, gap: f32) -> Vec<f32> {
    let mut order: Vec<usize> = (0..ys.len()).collect();
    order.sort_by(|&a, &b| ys[a].partial_cmp(&ys[b]).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = ys.clone();
    let mut last: Option<f32> = None;
    for i in order {
        let y = match last {
            Some(prev) if ys[i] < prev + gap => prev + gap,
            _ => ys[i],
        };
        out[i] = y;
        last = Some(y);
    }
    out
}

/// Where on the time line (design pixels, from the frame's top) a window last used `idle_ms` ago
/// sits: "now" at 140, "earlier" at 620.
pub fn time_y(idle_ms: u32) -> f32 {
    140.0 + timeline_pos(idle_ms) * 480.0
}

/// Each window's place on the time line, in design pixels: its time (`time_y`), spread so no two
/// names are closer than 22 px. In the windows' order.
pub fn line_ys(windows: &[Window]) -> Vec<f32> {
    spread(windows.iter().map(|w| time_y(w.idle_ms)).collect(), 22.0)
}

/// The time line's marks: label and position.
pub fn ticks() -> Vec<(&'static str, f32)> {
    vec![("now", 0.0), ("1 min", timeline_pos(60_000)), ("10 min", timeline_pos(600_000)), ("1 h", timeline_pos(3_600_000)), ("earlier", 1.0)]
}

/// The window type-to-jump goes to: the first, from the selected one onwards in labwc's order,
/// whose name or app id has a word starting with `typed` (case ignored). None when nothing
/// matches or nothing was typed.
pub fn jump(windows: &[Window], selected: usize, typed: &str) -> Option<usize> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() || windows.is_empty() {
        return None;
    }
    let n = windows.len();
    (0..n).map(|i| (selected + i) % n).find(|&i| {
        let w = &windows[i];
        let hay = format!("{} {}", name(w), clean(&w.app_id, 60)).to_lowercase();
        hay.split(|c: char| !c.is_alphanumeric()).any(|word| word.starts_with(&typed)) || hay.starts_with(&typed)
    })
}

/// The caption's second line: where the window is and when it was last used.
pub fn whereabouts(w: &Window) -> String {
    let mut parts = Vec::new();
    // The app, when the title does not already say it: "Notes · this desktop" for a note.
    let app = clean(&w.app_id, 40);
    let app = if app.is_empty() {
        our_app(&clean(&w.title, 120))
            .and_then(|id| yantrik_ui_kit::app_names::APP_NAMES.iter().find(|(a, _)| *a == id))
            .map(|(_, n)| n.to_string())
            .unwrap_or_default()
    } else {
        app
    };
    if !app.is_empty() && !name(w).to_lowercase().contains(&app.to_lowercase()) {
        parts.push(app);
    }
    let ws = clean(&w.workspace, 40);
    parts.push(if w.current_workspace { "this desktop".to_string() } else if ws.is_empty() { "another desktop".into() } else { ws });
    if w.minimized {
        parts.push("minimized".into());
    }
    if let Some(a) = ago(w.idle_ms) {
        parts.push(if a == "now" { "in use".into() } else { format!("used {a}") });
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(id: u32, title: &str, idle_ms: u32) -> Window {
        Window {
            id,
            app_id: String::new(),
            title: title.into(),
            workspace: "Workspace 1".into(),
            current_workspace: true,
            minimized: false,
            focused_at_start: false,
            idle_ms,
        }
    }

    #[test]
    fn the_selected_card_is_in_front_and_the_rest_step_back_in_labwcs_order() {
        assert_eq!(depth(2, 2, 5), 0);
        assert_eq!(depth(3, 2, 5), 1);
        assert_eq!(depth(1, 2, 5), 4, "the one before the selection is the deepest");
        let (front, back) = (place_at(0.0), place_at(4.0));
        assert!(back.w < front.w && back.x > front.x && back.y < front.y);
        assert_eq!((front.x, front.y, front.w, front.h), (80.0, 240.0, 640.0, 400.0));
    }

    #[test]
    fn a_title_cannot_draw_controls_or_turn_text_around() {
        assert_eq!(clean("Bank\u{202e}moc.live\u{1b}[2J\n  x", 80), "Bankmoc.live[2J x");
        assert_eq!(clean("  spaced \t out  ", 80), "spaced out");
        // Look-alikes: zero-width space, joiner, word joiner, soft hyphen, BOM, a tag character.
        assert_eq!(clean("Pay\u{200b}Pal\u{200d}\u{2060}\u{00ad}\u{feff}\u{e0041}", 80), "PayPal");
        assert_eq!(clean(&"a".repeat(200), 10).chars().count(), 10);
        assert!(clean(&"a".repeat(200), 10).ends_with('…'));
    }

    #[test]
    fn a_window_with_no_title_is_named_by_its_app_or_says_so() {
        let mut w = win(1, "  ", 0);
        assert_eq!(name(&w), "Untitled window");
        w.app_id = "foot".into();
        assert_eq!(name(&w), "foot");
        assert_eq!(icon_id(&w), "foot");
    }

    #[test]
    fn age_is_said_plainly_and_unknown_is_not_made_up() {
        assert_eq!(ago(0), None);
        assert_eq!(ago(5_000).as_deref(), Some("now"));
        assert_eq!(ago(4 * 60_000).as_deref(), Some("4 min ago"));
        assert_eq!(ago(60 * 60_000).as_deref(), Some("1 h ago"));
        assert_eq!(ago(135 * 60_000).as_deref(), Some("2 h 15 min ago"));
    }

    #[test]
    fn the_time_line_gives_recent_minutes_most_of_the_room() {
        let (one, ten, hour) = (timeline_pos(60_000), timeline_pos(600_000), timeline_pos(3_600_000));
        assert!(timeline_pos(0) == 0.0 && one > 0.0 && ten > one && hour > ten && hour < 1.0);
        assert!(one > 0.1, "the first minute is not squeezed to nothing: {one}");
        assert_eq!(timeline_pos(10 * 3_600_000), 1.0, "older than two hours is 'earlier'");
    }

    #[test]
    fn typing_jumps_to_the_next_window_with_a_word_that_starts_so() {
        let ws = vec![win(1, "Weather", 1), win(2, "Containers", 1), win(3, "plan.md · Tumble", 1)];
        assert_eq!(jump(&ws, 0, "tum"), Some(2));
        assert_eq!(jump(&ws, 0, "CON"), Some(1));
        assert_eq!(jump(&ws, 2, "w"), Some(0), "wraps round from the selection");
        assert_eq!(jump(&ws, 0, "zzz"), None);
        assert_eq!(jump(&ws, 0, ""), None);
    }

    #[test]
    fn the_caption_says_where_and_when() {
        let mut w = win(1, "Weather", 4 * 60_000);
        assert_eq!(whereabouts(&w), "this desktop · used 4 min ago");
        w.current_workspace = false;
        w.workspace = "Workspace 2".into();
        w.minimized = true;
        assert_eq!(whereabouts(&w), "Workspace 2 · minimized · used 4 min ago");
    }

    #[test]
    fn labels_at_the_same_moment_do_not_sit_on_each_other() {
        let out = spread(vec![120.0, 120.0, 300.0, 125.0], 18.0);
        assert_eq!(out[0], 120.0);
        assert_eq!(out[1], 138.0);
        assert_eq!(out[3], 156.0, "the third at 'now' goes below the second");
        assert_eq!(out[2], 300.0, "one far away is left alone");
    }

    #[test]
    fn a_window_of_ours_is_drawn_with_its_apps_icon() {
        assert_eq!(icon_id(&win(1, "System Monitor", 1)), "sysmonitor");
        assert_eq!(icon_id(&win(2, "Notes: Handover", 1)), "notes");
        assert_eq!(icon_id(&win(3, "plan.md - Editor", 1)), "editor");
        let mut c = win(4, "Yantrik OS - Chromium", 1);
        c.app_id = "chromium".into();
        assert_eq!(icon_id(&c), "browser");
        c.app_id = "yantrik-system-monitor".into();
        assert_eq!(icon_id(&c), "sysmonitor");
        assert_eq!(icon_id(&win(5, "labwc - WL-1", 1)), "~labwc - WL-1", "unknown: its title, for the initial");
        assert_eq!(icon_id(&win(6, "files", 1)), "~files", "a title cannot borrow one of our icons");
        assert_eq!(yantrik_ui_kit::app_tile::initial("~files"), "F", "and its initial is still its own");
    }

    #[test]
    fn a_note_says_which_app_it_is_in() {
        let w = win(1, "Handover - Editor", 2 * 60_000);
        assert_eq!(whereabouts(&w), "this desktop · used 2 min ago", "the title already says Editor");
        let w = win(1, "Notes", 2 * 60_000);
        assert_eq!(whereabouts(&w), "this desktop · used 2 min ago");
    }

    #[test]
    fn the_slots_are_the_art_directions() {
        let want = [
            (80.0, 240.0, 640.0, 400.0),
            (224.0, 176.0, 576.0, 360.0),
            (352.0, 120.0, 512.0, 320.0),
            (464.0, 72.0, 448.0, 280.0),
            (560.0, 32.0, 384.0, 240.0),
        ];
        for (d, (x, y, w, h)) in want.into_iter().enumerate() {
            let p = place_at(d as f32);
            assert!((p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3 && (p.w - w).abs() < 1e-3 && (p.h - h).abs() < 1e-3, "depth {d}: {p:?}");
        }
        // Every card behind shows a band above and a strip to the right of the one in front of it.
        for d in 1..VISIBLE {
            let (a, b) = (place_at(d as f32 - 1.0), place_at(d as f32));
            assert!(b.y + 39.0 < a.y && b.x + b.w > a.x + a.w + 31.0, "depth {d} is hidden");
        }
        // Leaving past the front: a tenth bigger by -1, a little left and up.
        let out = place_at(-1.0);
        assert_eq!((out.x, out.y, out.w, out.h), (40.0, 226.0, 704.0, 440.0));
        let mid = place_at(0.5);
        assert!(mid.w < place_at(0.0).w && mid.w > place_at(1.0).w);
    }

    #[test]
    fn a_picture_keeps_its_shape_with_no_frame() {
        let slot = (80.0, 240.0, 640.0, 400.0);
        assert_eq!(fit(slot, 1.6, 0.0), slot, "a 16:10 window fills its slot");
        // A tall window in front: as tall as the slot, on its left edge.
        assert_eq!(fit(slot, 0.8, 0.0), (80.0, 240.0, 320.0, 400.0));
        // A wide one behind: as wide as the slot, at its top.
        assert_eq!(fit(slot, 3.2, 1.0), (80.0, 240.0, 640.0, 200.0));
        let (_, y, _, h) = fit(slot, 3.2, 0.0);
        assert_eq!(y + h, 640.0, "in front it sits on the slot's bottom");
        assert_eq!(fit(slot, f32::NAN, 0.0), slot);
    }

    #[test]
    fn only_what_the_card_in_front_leaves_showing_is_drawn() {
        let card = (224.0, 176.0, 576.0, 360.0);
        let front = (80.0, 240.0, 640.0, 400.0);
        let parts = uncovered(card, Some(front), 12.0);
        // A band along the top, down to 12 px under the front card's edge; a strip down the right.
        assert_eq!(parts, vec![(0.0, 0.0, 576.0, 76.0), (484.0, 76.0, 92.0, 284.0)]);
        let drawn: f32 = parts.iter().map(|p| p.2 * p.3).sum();
        assert!(drawn < 0.35 * card.2 * card.3, "most of it is hidden: {drawn}");
        assert_eq!(uncovered(card, None, 12.0), vec![(0.0, 0.0, 576.0, 360.0)]);
        assert_eq!(uncovered(card, Some((900.0, 0.0, 100.0, 100.0)), 12.0), vec![(0.0, 0.0, 576.0, 360.0)], "not touching");
        // Covered on all sides but a hole: four pieces round it.
        assert_eq!(uncovered((0.0, 0.0, 100.0, 100.0), Some((28.0, 28.0, 44.0, 44.0)), 12.0).len(), 4);
    }

    #[test]
    fn the_marker_glides_and_is_retargeted_from_where_it_is() {
        let mut m = Tween::at(140.0, 0.18);
        assert!(m.done());
        m.retarget(300.0);
        m.step(0.09);
        let half = m.value();
        assert!(half > 220.0 && half < 300.0, "eases out: {half}");
        m.retarget(140.0);
        assert_eq!(m.value(), half, "no jump on a new target");
        m.step(0.2);
        assert!(m.done() && m.value() == 140.0);
    }

    fn run(sp: &mut Spring, seconds: f32) {
        let mut t = 0.0;
        while t < seconds {
            sp.step(0.016, 22.0, 0.16, 4);
            t += 0.016;
        }
    }

    #[test]
    fn a_card_springs_to_its_place_without_overshoot() {
        let mut sp = Spring::at(1.0);
        sp.retarget(0.0, 4);
        let mut lowest = f32::MAX;
        for _ in 0..40 {
            sp.step(0.016, 22.0, 0.16, 4);
            lowest = lowest.min(sp.pos);
        }
        assert!(lowest > -0.01, "overshot to {lowest}");
        assert!((sp.pos - 0.0).abs() < 0.01);
    }

    #[test]
    fn tab_sends_the_front_card_out_forward_and_back_in_at_the_back() {
        // 4 windows, Tab: the front card (0) now belongs at 3. It leaves toward -1 first.
        let mut sp = Spring::at(0.0);
        sp.retarget(3.0, 4);
        assert!(sp.target < 0.0 && sp.leaving == Some(3.0));
        run(&mut sp, 1.0);
        assert!(sp.leaving.is_none() && (sp.pos - 3.0).abs() < 0.01 && sp.appear == 1.0);
    }

    #[test]
    fn a_jump_glides_straight_and_stays_solid() {
        // Typing a name three back in five: that card comes straight forward, through the stack.
        let mut sp = Spring::at(3.0);
        sp.glide_to(0.0);
        let mut t = 0.0;
        while t < 1.0 {
            sp.step(0.016, 15.0, 0.24, 5);
            assert!(sp.pos >= -1e-3 && fade(sp.pos) == 1.0, "went round, or faded: {}", sp.pos);
            t += 0.016;
        }
        assert!(sp.settled());
    }

    #[test]
    fn shift_tab_brings_the_back_card_in_from_the_front() {
        let mut sp = Spring::at(3.0);
        sp.retarget(0.0, 4);
        assert_eq!(sp.pos, -1.0);
        run(&mut sp, 1.0);
        assert!((sp.pos - 0.0).abs() < 0.01);
    }

    #[test]
    fn held_tab_keeps_cards_moving_without_stopping() {
        // A new target every 40 ms (key repeat): the card never stops between them.
        let mut sp = Spring::at(4.0);
        let mut slowest = f32::MAX;
        for k in 1..=4 {
            sp.retarget(4.0 - k as f32, 8);
            for _ in 0..3 {
                sp.step(0.0134, 22.0, 0.16, 8);
                slowest = slowest.min(sp.vel.abs());
            }
        }
        assert!(slowest > 1.0, "the card nearly stopped: {slowest}");
    }

    #[test]
    fn the_stack_is_solid_and_only_the_ends_fade() {
        assert_eq!(fade(0.0), 1.0);
        assert_eq!(fade(3.5), 1.0);
        assert_eq!(fade(-0.2), 1.0, "the leaving card is whole at first");
        assert_eq!(fade(-0.9), 0.0);
        assert_eq!(fade(4.5), 0.5);
        assert_eq!(dim(0.0), 0.0);
        assert!((dim(1.0) - 0.08).abs() < 1e-6 && (dim(4.0) - 0.28).abs() < 1e-6 && (dim(2.5) - 0.185).abs() < 1e-6);
        assert_eq!(dim(-0.5), 0.0);
        assert_eq!((chosen(0.0), chosen(-0.5), chosen(0.5)), (1.0, 1.0, 0.0));
    }

    #[test]
    fn a_held_tab_never_leaves_the_front_card_behind_the_selection() {
        // Soft spring (12), a new place every 40 ms for a second: it trails by at most a depth.
        let mut sp = Spring::at(20.0);
        for k in 1..=25 {
            sp.retarget(20.0 - k as f32, 64);
            for _ in 0..3 {
                sp.step(0.0134, 12.0, 0.24, 64);
                assert!((sp.pos - sp.target).abs() <= LAG_MOST + 1e-4);
            }
        }
    }


    #[test]
    fn a_step_is_sharp_by_a_fifth_of_a_second_and_done_by_half() {
        let mut sp = Spring::at(1.0);
        sp.retarget(0.0, 6);
        let (mut sharp_at, mut done_at, mut t) = (None, None, 0.0f32);
        while t < 1.0 {
            sp.step(0.016, 15.0, 0.24, 6);
            t += 0.016;
            if sharp_at.is_none() && !sp.fast() {
                sharp_at = Some(t);
            }
            if done_at.is_none() && sp.settled() {
                done_at = Some(t);
            }
        }
        let (sharp, done) = (sharp_at.unwrap(), done_at.unwrap());
        assert!(sharp < 0.25, "full size only from {sharp} s");
        assert!(done < 0.5, "settled only at {done} s");
    }
}
