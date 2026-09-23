//! Choosing a Slint renderer that will not cook the CPU — and drawing on the GPU when there is one.
//!
//! Both renderers are compiled in (see the workspace `slint` features) and Slint picks between
//! them from `SLINT_BACKEND` at startup. Picking wrong is expensive in a way that is not obvious,
//! so the choice is made here rather than left to whoever wrote the launch script.
//!
//! ## Measured, on WSLg with an RTX 3090 Ti reachable
//!
//! Average CPU of the shell over a 20-second window on the animated desktop:
//!
//! | Configuration                              | CPU    |
//! |--------------------------------------------|--------|
//! | `winit-femtovg` + `GALLIUM_DRIVER=d3d12`   |  41 %  |
//! | `winit-software`                           |  98 %  |
//! | `winit-femtovg` with no GPU (llvmpipe)     | 576 %  |
//!
//! The top row is the win. The bottom row is the trap: femtovg asks OpenGL to do the drawing, and
//! when Mesa answers with llvmpipe, "OpenGL" is a multi-threaded software rasteriser that will
//! happily saturate six cores doing what Slint's own single-threaded rasteriser does in one.
//! Selecting the GPU renderer on a machine that has no usable GPU is not a mild misconfiguration;
//! it is six times *worse* than the thing it replaced. An unknown machine gets the safe 98 %,
//! never the 576 %.
//!
//! ## Who decides: the session, once, before anything draws
//!
//! The question "does this machine have a GPU that works" is answered by `yantrik-session`
//! (deploy/yantrik-os/yantrik-session), before labwc starts, because the compositor has to be told
//! as well as the shell and it chooses its renderer as it starts. The session asks Mesa
//! (`eglinfo -B -p gbm`, under a timeout) for the OpenGL ES renderer; llvmpipe, softpipe, swrast or
//! no answer means software. Two combinations are software even though the renderer looks like
//! hardware: vmwgfx on a hypervisor that is not VMware, and virtio-gpu without virgl. Everything
//! else is the GPU. It exports the verdict as `YANTRIK_GRAPHICS=gpu|software`, with
//! `YANTRIK_GRAPHICS_SOURCE`, `_REASON` and `_RENDERER`, and writes the same to
//! `$XDG_RUNTIME_DIR/yantrik/graphics`. This file follows it — femtovg when the session says GPU,
//! Slint's software renderer when it says software — reading the record before the environment,
//! because `YANTRIK_GRAPHICS` is also the knob a person sets, and labwc's environment file is
//! applied over what labwc inherits.
//!
//! This replaced a rule that lived here: femtovg when a DRM render node's kernel driver was on an
//! allow-list (amdgpu, i915, nouveau…). A driver's *name* cannot tell you whether acceleration is
//! real — virtio-gpu publishes a render node and then Mesa falls back to llvmpipe (272 % CPU on a
//! Proxmox VM). And the list was never consulted on an installed machine anyway: every image wrote
//! `LIBGL_ALWAYS_SOFTWARE=1` into the labwc environment, so every install, whatever its hardware,
//! drew on the CPU with its ambient animation switched off. The allow-list survives only for a
//! shell started with no session verdict at all (a developer's run, WSL), where it is still the
//! conservative answer.
//!
//! ## VirtualBox: "Mesa says hardware" is not proof either
//!
//! Found live on 2026-09-23. VirtualBox's VMSVGA adapter with 3D enabled, Debian 13, kernel 6.12:
//! `eglinfo -B -p gbm` reports `SVGA3D; build: RELEASE; LLVM;`, and Mesa really does accelerate
//! it. The desktop still cannot use it. With labwc on `WLR_RENDERER=gles2` and the shell on
//! `winit-femtovg`, labwc fails "importing the supplied dmabufs failed" on the shell's first frame
//! (with or without `WLR_DRM_NO_MODIFIERS=1`), the compositor drops the shell's connection, and
//! the shell's event loop ends. The kernel had said as much: "vmwgfx seems to be running on an
//! unsupported hypervisor … likely broken". So the session treats vmwgfx outside VMware as
//! software up front, and treats every GPU verdict it reached by probing as a *trial*: if labwc or
//! the shell dies within 45 seconds, it records `~/.local/state/yantrik/graphics-fallback` (reason,
//! renderer string, build) and starts again in software straight away, and it does not try that
//! GPU again until the renderer string or the installed build changes. The shell's part in that
//! is to exit with a status that says its loop failed (`main.rs`, `EXIT_LOOP_FAILED`), and to show
//! what happened: `describe shell`'s `graphics` and Settings → About read [`for_describe`] and
//! [`about_lines`].
//!
//! ## What still overrides
//!
//! An explicit renderer in `SLINT_BACKEND` (`winit-software`, `winit-femtovg`, `winit-skia`) always
//! wins — a person or a launch script debugging a rendering problem can force either. A bare
//! `winit` names a window backend, not a renderer, and does not count. `LIBGL_ALWAYS_SOFTWARE=1`
//! means software even if the session said GPU: femtovg on that is the 576 % row. The session's own
//! overrides (`YANTRIK_GRAPHICS`, `WLR_RENDERER`, `yantrik.graphics=` on the kernel command line)
//! reach this file as its verdict.
//!
//! ## The WSL wrinkle
//!
//! Mesa does not probe `/dev/dxg` on its own — a plain `glxinfo` under WSLg reports llvmpipe and
//! `Accelerated: no` even though the d3d12 driver and the adapter are both right there. The GPU
//! only appears once `GALLIUM_DRIVER=d3d12` is set, so whenever femtovg is chosen on a machine with
//! `/dev/dxg` and Mesa's d3d12 driver, this sets it (unless the operator already has).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// What the session decided, as `yantrik-session` exported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionVerdict {
    /// `YANTRIK_GRAPHICS=gpu`.
    pub gpu: bool,
    /// How: `probe`, `known-broken`, `fallback` or `override`.
    pub source: String,
    /// The session's own sentence.
    pub reason: String,
    /// The OpenGL ES renderer Mesa reported, or empty when nothing answered.
    pub renderer: String,
}

impl SessionVerdict {
    /// From the four values, or `None` when the verdict is not one the session writes.
    fn from_parts(verdict: &str, source: &str, reason: &str, renderer: &str) -> Option<Self> {
        let gpu = match verdict.trim() {
            "gpu" => true,
            "software" => false,
            _ => return None,
        };
        Some(SessionVerdict {
            gpu,
            source: source.trim().to_string(),
            reason: reason.trim().to_string(),
            renderer: renderer.trim().to_string(),
        })
    }

    /// From this process's environment, which the session exported to labwc and labwc to us.
    /// Second to the record: see [`Inputs::from_machine`].
    fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        Self::from_parts(
            &var("YANTRIK_GRAPHICS"),
            &var("YANTRIK_GRAPHICS_SOURCE"),
            &var("YANTRIK_GRAPHICS_REASON"),
            &var("YANTRIK_GRAPHICS_RENDERER"),
        )
    }

    /// From the session's record, `$XDG_RUNTIME_DIR/yantrik/graphics`: `key=value` lines, written
    /// by `yantrik-session` for the start it is making.
    fn from_record(text: &str) -> Option<Self> {
        let fields = key_values(text);
        let get = |k: &str| fields.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str()).unwrap_or("");
        Self::from_parts(get("verdict"), get("source"), get("reason"), get("renderer"))
    }

    fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "verdict": if self.gpu { "gpu" } else { "software" },
            "source": self.source,
            "reason": self.reason,
            "renderer": self.renderer,
        })
    }
}

/// `key=value` lines, first `=` splits, `#` lines and blank lines skipped.
fn key_values(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.to_string()))
        .collect()
}

/// The record `yantrik-session` leaves when the GPU failed in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallback {
    pub reason: String,
    pub renderer: String,
    pub build: String,
    pub when: String,
}

impl Fallback {
    fn parse(text: &str) -> Option<Self> {
        let fields = key_values(text);
        let get = |k: &str| {
            fields
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default()
        };
        let reason = get("reason");
        if reason.is_empty() {
            return None;
        }
        Some(Fallback {
            reason,
            renderer: get("renderer"),
            build: get("build"),
            when: get("when"),
        })
    }
}

/// Where the session keeps the fallback record: `$XDG_STATE_HOME/yantrik/graphics-fallback`,
/// `~/.local/state/…` by default — the same path the script writes.
pub fn fallback_path() -> Option<PathBuf> {
    let state = match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(state.join("yantrik/graphics-fallback"))
}

fn read_fallback() -> Option<Fallback> {
    Fallback::parse(&std::fs::read_to_string(fallback_path()?).ok()?)
}

fn read_session_record() -> Option<SessionVerdict> {
    let run = std::env::var_os("XDG_RUNTIME_DIR")?;
    SessionVerdict::from_record(&std::fs::read_to_string(Path::new(&run).join("yantrik/graphics")).ok()?)
}

/// What made the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    /// `SLINT_BACKEND` named a renderer.
    Explicit,
    /// The session's verdict.
    Session,
    /// `LIBGL_ALWAYS_SOFTWARE=1`, which overrules a GPU verdict.
    LibglSoftware,
    /// No session verdict: this file's own conservative detection.
    Detection,
}

impl DecidedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            DecidedBy::Explicit => "SLINT_BACKEND",
            DecidedBy::Session => "session",
            DecidedBy::LibglSoftware => "LIBGL_ALWAYS_SOFTWARE",
            DecidedBy::Detection => "detection",
        }
    }
}

/// Everything the choice depended on, so it can be made without touching the machine.
#[derive(Debug, Default, Clone)]
struct Inputs {
    /// `SLINT_BACKEND`, only when it names a renderer.
    explicit_backend: Option<String>,
    libgl_software: bool,
    session: Option<SessionVerdict>,
    /// `/dev/dxg` and Mesa's d3d12 driver are both present.
    wsl_d3d12: bool,
    /// The kernel driver behind the first DRM render node.
    render_driver: Option<String>,
}

impl Inputs {
    fn from_machine() -> Self {
        let explicit_backend = std::env::var("SLINT_BACKEND")
            .ok()
            .filter(|b| has_explicit_renderer(b));
        Inputs {
            explicit_backend,
            libgl_software: matches!(
                std::env::var("LIBGL_ALWAYS_SOFTWARE").as_deref(),
                Ok("1") | Ok("true")
            ),
            // The record first. `YANTRIK_GRAPHICS` is also the knob a person sets, and labwc applies
            // ~/.config/labwc/environment over what it inherits — so a `YANTRIK_GRAPHICS=gpu` line
            // there reaches this process even on a start that `nomodeset` made software. The record
            // is what the session decided for this start; the environment is the fallback for a
            // shell that has none to read.
            session: read_session_record().or_else(SessionVerdict::from_env),
            wsl_d3d12: Path::new("/dev/dxg").exists() && has_gallium_driver("d3d12"),
            render_driver: render_node_driver(),
        }
    }
}

/// What we believe about the graphics stack, and why.
#[derive(Debug, Clone)]
struct Verdict {
    backend: String,
    /// Set `GALLIUM_DRIVER` to this before Slint initialises, if it is not already set.
    gallium: Option<&'static str>,
    reason: String,
    decided_by: DecidedBy,
}

impl Verdict {
    fn software(reason: impl Into<String>, decided_by: DecidedBy) -> Self {
        Verdict {
            backend: "winit-software".into(),
            gallium: None,
            reason: reason.into(),
            decided_by,
        }
    }

    fn femtovg(wsl_d3d12: bool, reason: impl Into<String>, decided_by: DecidedBy) -> Self {
        Verdict {
            backend: "winit-femtovg".into(),
            gallium: wsl_d3d12.then_some("d3d12"),
            reason: reason.into(),
            decided_by,
        }
    }
}

/// The decision itself, free of side effects so it can be reasoned about and tested.
fn choose(i: &Inputs) -> Verdict {
    if let Some(backend) = &i.explicit_backend {
        return Verdict {
            backend: backend.clone(),
            gallium: None,
            reason: format!("SLINT_BACKEND={backend} was set explicitly"),
            decided_by: DecidedBy::Explicit,
        };
    }

    if let Some(session) = &i.session {
        if !session.gpu {
            return Verdict::software(
                format!("the session chose software: {}", session.reason),
                DecidedBy::Session,
            );
        }
        if i.libgl_software {
            // Only a person can have put this here on a GPU session — the session clears it —
            // and femtovg on it is the 576 % row.
            return Verdict::software(
                "the session chose the GPU, but LIBGL_ALWAYS_SOFTWARE=1 puts OpenGL on llvmpipe, \
                 where Slint's own rasteriser is far cheaper",
                DecidedBy::LibglSoftware,
            );
        }
        return Verdict::femtovg(
            i.wsl_d3d12,
            format!("the session chose the GPU: {}", session.reason),
            DecidedBy::Session,
        );
    }

    // No session verdict: a shell started some other way. Stay conservative.
    if i.libgl_software {
        return Verdict::software(
            "LIBGL_ALWAYS_SOFTWARE=1: software graphics requested; avoid software OpenGL",
            DecidedBy::LibglSoftware,
        );
    }
    // WSL2: the GPU is behind /dev/dxg, and Mesa reaches it through the d3d12 Gallium driver only
    // when told to. The presence of the node plus the driver is sufficient evidence.
    if i.wsl_d3d12 {
        return Verdict::femtovg(
            true,
            "no session verdict; WSL /dev/dxg present with the d3d12 Mesa driver",
            DecidedBy::Detection,
        );
    }
    if let Some(driver) = &i.render_driver {
        if accelerates(driver) {
            return Verdict::femtovg(
                false,
                format!("no session verdict; DRM render node with the {driver} driver"),
                DecidedBy::Detection,
            );
        }
    }
    Verdict::software(
        "no session verdict and no GPU found — the software rasteriser is far cheaper than \
         software OpenGL",
        DecidedBy::Detection,
    )
}

/// The decision against this machine, without exporting anything.
#[cfg(test)]
fn decide() -> Verdict {
    choose(&Inputs::from_machine())
}

/// What the shell settled on, for the places that report it.
#[derive(Debug, Clone)]
pub struct Choice {
    pub renderer: Renderer,
    pub backend: String,
    pub reason: String,
    pub decided_by: DecidedBy,
    pub session: Option<SessionVerdict>,
    pub fallback: Option<Fallback>,
    /// The session fell back in the start that launched this shell (not an earlier one).
    pub fallback_new: bool,
}

static CHOICE: OnceLock<Choice> = OnceLock::new();

/// What [`select`] chose, once it has run.
pub fn choice() -> Option<&'static Choice> {
    CHOICE.get()
}

/// Decide the renderer and export the environment Slint reads.
///
/// Must be called before the first Slint call — `App::new()` reads `SLINT_BACKEND` and never looks
/// again. An explicit renderer in `SLINT_BACKEND` always wins; otherwise the session's verdict;
/// otherwise this file's own conservative detection. See the module documentation.
pub fn select() -> Renderer {
    let inputs = Inputs::from_machine();
    let v = choose(&inputs);

    if let Some(driver) = v.gallium {
        // Only fill this in if the operator has not expressed an opinion.
        if std::env::var("GALLIUM_DRIVER").is_err() {
            std::env::set_var("GALLIUM_DRIVER", driver);
            tracing::info!(driver, "Set GALLIUM_DRIVER so Mesa finds the GPU");
        }
    }
    if v.decided_by != DecidedBy::Explicit {
        std::env::set_var("SLINT_BACKEND", &v.backend);
    }

    let renderer = Renderer::from_backend(&v.backend);
    tracing::info!(
        backend = %v.backend,
        decided_by = v.decided_by.as_str(),
        reason = %v.reason,
        session_renderer = inputs.session.as_ref().map(|s| s.renderer.as_str()).unwrap_or(""),
        "Renderer selected"
    );
    let _ = CHOICE.set(Choice {
        renderer,
        backend: v.backend,
        reason: v.reason,
        decided_by: v.decided_by,
        session: inputs.session,
        fallback: read_fallback(),
        fallback_new: std::env::var("YANTRIK_GRAPHICS_FALLBACK_NEW").as_deref() == Ok("1"),
    });
    renderer
}

impl Choice {
    fn as_json(&self) -> serde_json::Value {
        let drawing = match self.renderer {
            Renderer::Gpu => "gpu",
            Renderer::Cpu => "software",
        };
        serde_json::json!({
            // What the shell draws with: "gpu" or "software".
            "drawing": drawing,
            "backend": self.backend,
            "decided_by": self.decided_by.as_str(),
            "reason": self.reason,
            // What Mesa reported to the session, whichever way it went.
            "renderer": self.session.as_ref().map(|s| s.renderer.clone()),
            "session": self.session.as_ref().map(SessionVerdict::as_json),
            // Null when the shell was not started by a session that decided.
            "matches_session": self.session.as_ref().map(|s| s.gpu == (self.renderer == Renderer::Gpu)),
            // The GPU failed in use here before; the session keeps the record.
            "fallback": self.fallback.as_ref().map(|f| serde_json::json!({
                "reason": f.reason,
                "renderer": f.renderer,
                "build": f.build,
                "when": f.when,
                "record": fallback_path().map(|p| p.display().to_string()),
            })),
        })
    }

    /// The two lines Settings → About shows: what draws, and why.
    fn about(&self) -> (String, String) {
        let renderer = self
            .session
            .as_ref()
            .map(|s| s.renderer.as_str())
            .filter(|r| !r.is_empty());
        let headline = match (self.renderer, renderer) {
            (Renderer::Gpu, Some(r)) => format!("GPU · {r}"),
            (Renderer::Gpu, None) => "GPU".to_string(),
            (Renderer::Cpu, _) => "Software".to_string(),
        };
        let mut detail = capitalise(&self.reason);
        if let (Some(f), Some(s)) = (&self.fallback, &self.session) {
            // The session's own sentence already says it when this start is the fallback; say it
            // here only when an earlier record exists and the session went another way (an
            // override), so the person can see why it might go back to software.
            if s.source != "fallback" {
                detail.push_str(&format!(
                    ". The GPU failed here before ({}), recorded {}",
                    f.reason, f.when
                ));
            }
        }
        (headline, detail)
    }
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `describe shell`'s `graphics`: what the shell draws with, who decided, the session's verdict
/// and any fallback. Null before [`select`] has run, which in the shell is never.
pub fn for_describe() -> serde_json::Value {
    choice().map(Choice::as_json).unwrap_or(serde_json::Value::Null)
}

/// Settings → About's Graphics row: a headline and a sentence.
pub fn about_lines() -> (String, String) {
    choice()
        .map(Choice::about)
        .unwrap_or_else(|| ("Unknown".to_string(), String::new()))
}

/// Say once, where the person will see it, that this desktop fell back to software just now.
///
/// Only on the start the fallback happened in: the next login is software from the record and
/// has nothing new to say; About and `describe` carry it from then on.
pub fn announce_fallback() {
    let Some(c) = choice() else { return };
    if !c.fallback_new || c.renderer != Renderer::Cpu {
        return;
    }
    let why = c
        .fallback
        .as_ref()
        .map(|f| f.reason.clone())
        .unwrap_or_else(|| c.reason.clone());
    yantrik_app_runtime::notify::send(
        yantrik_app_runtime::notify::Notification::new(
            "Graphics",
            "The GPU failed, so the desktop is drawing in software",
        )
        .body(format!(
            "{}. It will try the GPU again after the next update or on new hardware. Settings → About has the details.",
            capitalise(&why)
        )),
    );
}

// `winit` selects a window backend, not a renderer. Leaving it alone lets Slint
// choose OpenGL even on a VM, while our animation policy incorrectly assumes CPU.
fn has_explicit_renderer(backend: &str) -> bool {
    !matches!(backend.trim(), "" | "winit")
}

/// Which drawing path the shell ended up on — the thing callers actually want to branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// Drawing on the GPU. Frames are close to free.
    Gpu,
    /// Drawing on the CPU. Every frame costs real time on the main thread.
    Cpu,
}

impl Renderer {
    fn from_backend(backend: &str) -> Self {
        if backend.contains("femtovg") || backend.contains("skia") {
            Renderer::Gpu
        } else {
            Renderer::Cpu
        }
    }

    /// Milliseconds between ambient animation ticks. `0` disables ambient animation.
    ///
    /// ## Why the CPU answer is "off", not "slower"
    ///
    /// Measured on this shell, fullscreen, software rasteriser: an idle screen with no animation
    /// costs **0.6 %** of a core, and the animated desktop costs **97 %**. Rendering is entirely
    /// on demand — Slint is not burning anything until something asks it to repaint.
    ///
    /// The cost is per frame, and the frame is expensive: one full-screen software repaint of this
    /// desktop takes roughly 96 ms. That number is the whole story. At 60fps the shell was asking
    /// for a frame every 16 ms and finishing one every 96, so it thrashed at ~10fps and a pinned
    /// core. Dropping the request to 100 ms removed the thrash and changed the CPU by almost
    /// nothing, because 10fps of a 96 ms frame is still a saturated core.
    ///
    /// There is no interval that makes an expensive frame cheap. So on the CPU path the ambient
    /// decoration — orb, particles, drifting backdrop — is off by default, and the desktop costs
    /// what an idle screen costs. The features people actually use are the apps and the companion,
    /// not a breathing gradient; spending an entire core on the gradient is what made both feel
    /// slow. On a GPU the same frame is nearly free, so it runs at 60fps there.
    ///
    /// `YANTRIK_AMBIENT_MS` overrides this — set it to tune or to re-enable motion on a CPU box
    /// that has cores to spare.
    pub fn ambient_interval_ms(self) -> i32 {
        if let Ok(raw) = std::env::var("YANTRIK_AMBIENT_MS") {
            match raw.trim().parse::<i32>() {
                Ok(ms) if ms >= 0 => {
                    tracing::info!(ms, "Ambient interval overridden by YANTRIK_AMBIENT_MS");
                    return ms;
                }
                _ => tracing::warn!(
                    value = %raw,
                    "YANTRIK_AMBIENT_MS is not a non-negative integer; ignoring it"
                ),
            }
        }
        match self {
            Renderer::Gpu => 16,
            Renderer::Cpu => 0,
        }
    }
}

/// True if Mesa ships the named Gallium driver on this system.
///
/// Without the driver present, exporting `GALLIUM_DRIVER` would send Mesa looking for something
/// that is not there and land us back on llvmpipe — the worst of the three configurations.
fn has_gallium_driver(name: &str) -> bool {
    const DRI_DIRS: &[&str] = &[
        "/usr/lib/x86_64-linux-gnu/dri",
        "/usr/lib/dri",
        "/usr/lib64/dri",
    ];
    let file = format!("{name}_dri.so");
    DRI_DIRS.iter().any(|d| Path::new(d).join(&file).exists())
}

/// Which kernel driver is behind the first DRM render node, if there is one.
///
/// Only consulted with no session verdict. The node's existence is not evidence of acceleration:
/// virtio-pci, vmwgfx, qxl, bochs-drm, simpledrm and the mgag200/ast BMC chips all publish one.
fn render_node_driver() -> Option<String> {
    for entry in std::fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
    {
        if !entry.file_name().to_string_lossy().starts_with("renderD") {
            continue;
        }
        let uevent = std::fs::read_to_string(entry.path().join("device/uevent")).ok()?;
        if let Some(d) = uevent.lines().find_map(|l| l.strip_prefix("DRIVER=")) {
            return Some(d.trim().to_string());
        }
    }
    None
}

/// Whether that driver actually draws on hardware — the no-session fallback only.
///
/// Named as an allowlist rather than a blocklist: an unknown driver on a machine we have never
/// seen should land on the software rasteriser, which is merely slow, instead of software OpenGL,
/// which is slow AND makes the shell believe frames are free. The session's probe is the real
/// answer; this is what a shell started without one falls back on.
fn accelerates(driver: &str) -> bool {
    matches!(
        driver,
        "amdgpu"
            | "radeon"
            | "i915"
            | "xe"
            | "nouveau"
            | "nvidia"
            | "nvidia-drm"
            | "msm"
            | "panfrost"
            | "v3d"
    )
}

/// True if any DRM render node exists (`/dev/dri/renderD*`).
#[allow(dead_code)]
fn has_render_node() -> bool {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_str()
            .is_some_and(|n| n.starts_with("renderD"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(verdict: &str, source: &str, renderer: &str) -> Option<SessionVerdict> {
        SessionVerdict::from_parts(verdict, source, "because", renderer)
    }

    #[test]
    fn backend_only_setting_does_not_bypass_renderer_detection() {
        assert!(!has_explicit_renderer("winit"));
        assert!(!has_explicit_renderer(""));
        assert!(has_explicit_renderer("winit-software"));
        assert!(has_explicit_renderer("winit-femtovg"));
        assert!(has_explicit_renderer("winit-skia"));
        assert!(has_explicit_renderer("qt"));
    }

    #[test]
    fn a_machine_with_no_gpu_gets_the_software_rasteriser() {
        // The decision on this builder reflects whatever hardware and session it has, but the
        // invariant holds either way: femtovg is never chosen without a positive finding.
        let inputs = Inputs::from_machine();
        if inputs.explicit_backend.is_some() {
            return; // SLINT_BACKEND in the test's environment decides, by design.
        }
        let v = decide();
        if v.backend == "winit-femtovg" {
            assert!(
                inputs.session.as_ref().is_some_and(|s| s.gpu)
                    || Path::new("/dev/dxg").exists()
                    || has_render_node(),
                "femtovg was chosen with no GPU evidence — this is the 576% CPU case"
            );
        } else {
            assert_eq!(v.backend, "winit-software");
        }
        assert!(
            !v.reason.is_empty(),
            "every verdict explains itself in the log"
        );
    }

    #[test]
    fn gallium_is_only_requested_alongside_femtovg() {
        let v = decide();
        if v.gallium.is_some() {
            assert_eq!(
                v.backend, "winit-femtovg",
                "GALLIUM_DRIVER only matters to the GL renderer"
            );
        }
    }

    #[test]
    fn the_cpu_path_disables_ambient_decoration() {
        // Guard against the env override leaking in from the surrounding shell.
        if std::env::var("YANTRIK_AMBIENT_MS").is_ok() {
            return;
        }
        assert_eq!(
            Renderer::Gpu.ambient_interval_ms(),
            16,
            "60fps where frames are free"
        );
        assert_eq!(
            Renderer::Cpu.ambient_interval_ms(),
            0,
            "a 96ms software frame cannot be made cheap by asking for it less often"
        );
    }

    #[test]
    fn only_gl_backends_count_as_gpu() {
        assert_eq!(Renderer::from_backend("winit-femtovg"), Renderer::Gpu);
        assert_eq!(Renderer::from_backend("winit-skia"), Renderer::Gpu);
        assert_eq!(Renderer::from_backend("winit-software"), Renderer::Cpu);
        // Anything unrecognised must be treated as the CPU path: guessing "GPU" for an unknown
        // backend is how a machine ends up drawing decoration at 60fps on a rasteriser.
        assert_eq!(Renderer::from_backend("something-new"), Renderer::Cpu);
    }

    #[test]
    fn a_missing_driver_is_not_reported_as_present() {
        assert!(!has_gallium_driver("definitely-not-a-real-driver"));
    }

    // ── The session's verdict ──

    #[test]
    fn a_gpu_session_draws_with_femtovg() {
        let v = choose(&Inputs {
            session: session("gpu", "probe", "Mesa Intel(R) UHD Graphics 620 (KBL GT2)"),
            ..Inputs::default()
        });
        assert_eq!(v.backend, "winit-femtovg");
        assert_eq!(v.decided_by, DecidedBy::Session);
        assert_eq!(v.gallium, None, "only WSL needs Mesa pointed at d3d12");
    }

    #[test]
    fn a_software_session_draws_in_software_whatever_the_render_node_says() {
        // The VirtualBox machine: vmwgfx publishes a render node, but the session said software.
        // So does an amdgpu machine whose GPU failed in use — the allow-list must not overrule it.
        for source in ["probe", "known-broken", "fallback", "override"] {
            let v = choose(&Inputs {
                session: session("software", source, "SVGA3D; build: RELEASE;  LLVM;"),
                render_driver: Some("amdgpu".into()),
                ..Inputs::default()
            });
            assert_eq!(v.backend, "winit-software", "source {source}");
            assert_eq!(v.decided_by, DecidedBy::Session);
        }
    }

    #[test]
    fn a_gpu_session_needs_no_driver_on_an_allow_list() {
        // What the old rule got wrong in the other direction: a GPU the list did not name.
        let v = choose(&Inputs {
            session: session("gpu", "probe", "SVGA3D; build: RELEASE;  LLVM;"),
            render_driver: Some("vmwgfx".into()),
            ..Inputs::default()
        });
        assert_eq!(v.backend, "winit-femtovg");
    }

    #[test]
    fn a_gpu_session_on_wsl_points_mesa_at_d3d12() {
        let v = choose(&Inputs {
            session: session("gpu", "probe", "D3D12 (NVIDIA GeForce RTX 3090 Ti)"),
            wsl_d3d12: true,
            ..Inputs::default()
        });
        assert_eq!((v.backend.as_str(), v.gallium), ("winit-femtovg", Some("d3d12")));
    }

    #[test]
    fn an_explicit_renderer_beats_the_session() {
        for (backend, session_verdict) in [("winit-software", "gpu"), ("winit-femtovg", "software")] {
            let v = choose(&Inputs {
                explicit_backend: Some(backend.into()),
                session: session(session_verdict, "probe", "x"),
                ..Inputs::default()
            });
            assert_eq!(v.backend, backend);
            assert_eq!(v.decided_by, DecidedBy::Explicit);
        }
    }

    #[test]
    fn libgl_always_software_overrules_a_gpu_session() {
        // femtovg on LIBGL_ALWAYS_SOFTWARE is llvmpipe: the 576 % row.
        let v = choose(&Inputs {
            libgl_software: true,
            session: session("gpu", "probe", "Mesa Intel(R) UHD Graphics 620 (KBL GT2)"),
            ..Inputs::default()
        });
        assert_eq!(v.backend, "winit-software");
        assert_eq!(v.decided_by, DecidedBy::LibglSoftware);
    }

    #[test]
    fn without_a_session_the_old_conservative_rule_still_applies() {
        let d = |i: Inputs| choose(&i);
        assert_eq!(d(Inputs::default()).backend, "winit-software");
        assert_eq!(
            d(Inputs { render_driver: Some("virtio_gpu".into()), ..Inputs::default() }).backend,
            "winit-software",
            "virtio-gpu makes a render node and then falls back to llvmpipe"
        );
        assert_eq!(
            d(Inputs { render_driver: Some("i915".into()), ..Inputs::default() }).backend,
            "winit-femtovg"
        );
        let wsl = d(Inputs { wsl_d3d12: true, ..Inputs::default() });
        assert_eq!((wsl.backend.as_str(), wsl.gallium), ("winit-femtovg", Some("d3d12")));
        assert_eq!(
            d(Inputs { libgl_software: true, wsl_d3d12: true, ..Inputs::default() }).backend,
            "winit-software"
        );
        assert_eq!(d(Inputs::default()).decided_by, DecidedBy::Detection);
    }

    #[test]
    fn a_verdict_the_session_does_not_write_is_no_verdict() {
        assert!(SessionVerdict::from_parts("", "probe", "", "").is_none());
        assert!(SessionVerdict::from_parts("auto", "probe", "", "").is_none());
        assert!(SessionVerdict::from_parts("gpu", "probe", "", "").is_some_and(|s| s.gpu));
        assert!(SessionVerdict::from_parts(" software ", "probe", "", "").is_some_and(|s| !s.gpu));
    }

    #[test]
    fn the_session_record_is_read_as_the_script_writes_it() {
        let record = "verdict=software\nsource=known-broken\n\
            reason=vmwgfx outside VMware (oracle innotek GmbH): Mesa reports SVGA3D; build: RELEASE;  LLVM;, but labwc cannot import the shell's GPU buffers there\n\
            renderer=SVGA3D; build: RELEASE;  LLVM;\nbuild=v0.2.0-1-gaaaaaaa\ncompositor_renderer=pixman\n\
            started=2026-09-23T12:00:00Z\n";
        let s = SessionVerdict::from_record(record).expect("a record the script writes parses");
        assert!(!s.gpu);
        assert_eq!(s.source, "known-broken");
        assert_eq!(s.renderer, "SVGA3D; build: RELEASE;  LLVM;");
        assert!(s.reason.starts_with("vmwgfx outside VMware"), "{}", s.reason);
        assert!(SessionVerdict::from_record("").is_none());
    }

    #[test]
    fn the_fallback_record_is_read_as_the_script_writes_it() {
        let record = "# Written by yantrik-session: the desktop failed on this machine's GPU, so it draws in software.\n\
            # The GPU is tried again by itself when the renderer or the build below changes.\n\
            renderer=SVGA3D; build: RELEASE;  LLVM;\n\
            build=v0.2.0-1-gaaaaaaa\n\
            reason=the shell exited 3s after it started (status 70: its event loop failed while the compositor was still running)\n\
            when=2026-09-23T12:00:00Z\n";
        let f = Fallback::parse(record).expect("the script's record parses");
        assert_eq!(f.renderer, "SVGA3D; build: RELEASE;  LLVM;");
        assert_eq!(f.build, "v0.2.0-1-gaaaaaaa");
        assert!(f.reason.contains("status 70"));
        assert_eq!(f.when, "2026-09-23T12:00:00Z");
        assert!(Fallback::parse("# nothing but comments\n").is_none(), "a record says why or is no record");
    }

    #[test]
    fn describe_says_what_draws_who_decided_and_whether_it_matches() {
        let choice = Choice {
            renderer: Renderer::Cpu,
            backend: "winit-software".into(),
            reason: "the session chose software: the GPU failed here before".into(),
            decided_by: DecidedBy::Session,
            session: session("software", "fallback", "SVGA3D; build: RELEASE;  LLVM;"),
            fallback: Fallback::parse("reason=the shell exited 3s after it started\nrenderer=SVGA3D\nbuild=b\nwhen=t\n"),
            fallback_new: true,
        };
        let j = choice.as_json();
        assert_eq!(j["drawing"], "software");
        assert_eq!(j["backend"], "winit-software");
        assert_eq!(j["decided_by"], "session");
        assert_eq!(j["renderer"], "SVGA3D; build: RELEASE;  LLVM;");
        assert_eq!(j["session"]["verdict"], "software");
        assert_eq!(j["session"]["source"], "fallback");
        assert_eq!(j["matches_session"], true);
        assert_eq!(j["fallback"]["reason"], "the shell exited 3s after it started");

        let mismatch = Choice {
            renderer: Renderer::Gpu,
            backend: "winit-femtovg".into(),
            decided_by: DecidedBy::Explicit,
            fallback: None,
            ..choice.clone()
        };
        assert_eq!(mismatch.as_json()["matches_session"], false);
        assert!(mismatch.as_json()["fallback"].is_null());

        let sessionless = Choice { session: None, ..choice };
        assert!(sessionless.as_json()["matches_session"].is_null());
        assert!(sessionless.as_json()["renderer"].is_null());
    }

    #[test]
    fn about_names_the_renderer_and_the_reason() {
        let gpu = Choice {
            renderer: Renderer::Gpu,
            backend: "winit-femtovg".into(),
            reason: "the session chose the GPU: Mesa reports a hardware renderer: Mesa Intel(R) UHD Graphics 620 (KBL GT2)".into(),
            decided_by: DecidedBy::Session,
            session: session("gpu", "probe", "Mesa Intel(R) UHD Graphics 620 (KBL GT2)"),
            fallback: None,
            fallback_new: false,
        };
        let (headline, detail) = gpu.about();
        assert_eq!(headline, "GPU · Mesa Intel(R) UHD Graphics 620 (KBL GT2)");
        assert!(detail.starts_with("The session chose the GPU"), "{detail}");

        // Forced back onto the GPU over an old record: the record is still worth a sentence.
        let forced = Choice {
            session: session("gpu", "override", "SVGA3D; build: RELEASE;  LLVM;"),
            fallback: Fallback::parse("reason=the shell exited 3s after it started\nwhen=2026-09-23T12:00:00Z\n"),
            ..gpu.clone()
        };
        assert!(forced.about().1.contains("The GPU failed here before"), "{}", forced.about().1);

        let software = Choice { renderer: Renderer::Cpu, backend: "winit-software".into(), ..gpu };
        assert_eq!(software.about().0, "Software");
    }

    /// The session script's own table — recorded eglinfo output for Intel, AMD, NVIDIA, llvmpipe,
    /// SVGA3D on VirtualBox and on VMware, virgl, virtio without virgl and d3d12 on WSL, the
    /// overrides, the fallback record's whole life and the migration of an old environment file.
    /// It needs no GPU; running it here means `cargo test -p yantrik-ui` covers the half of the
    /// rule that lives in shell as well as the half that lives in this file.
    #[cfg(unix)]
    #[test]
    fn session_script_selftest_passes() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/yantrik-session");
        let out = std::process::Command::new("sh")
            .arg(&script)
            .arg("selftest")
            .output()
            .expect("sh runs");
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "yantrik-session selftest failed:\n{text}");
        assert!(text.contains("0 failed"), "{text}");
    }
}
