//! Choosing a Slint renderer that will not cook the CPU.
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
//! The middle row is what the shell shipped with. The top row is the win. The bottom row is the
//! trap, and it is the reason this file exists: femtovg asks OpenGL to do the drawing, and when
//! Mesa answers with llvmpipe, "OpenGL" is a multi-threaded software rasteriser that will happily
//! saturate six cores doing what Slint's own single-threaded rasteriser does in one. Selecting the
//! GPU renderer on a machine that has no usable GPU is therefore not a mild misconfiguration; it
//! is six times *worse* than the thing it replaced.
//!
//! So the rule is: use femtovg only on positive evidence of hardware, and fall back to the
//! software renderer whenever that evidence is missing. An unknown environment gets the safe 98 %,
//! never the 576 %.
//!
//! ## The WSL wrinkle
//!
//! Mesa does not probe `/dev/dxg` on its own — a plain `glxinfo` under WSLg reports llvmpipe and
//! `Accelerated: no` even though the d3d12 driver and the adapter are both right there. The GPU
//! only appears once `GALLIUM_DRIVER=d3d12` is set, so on WSL this function sets it. That is why
//! the check is for the *device node* rather than for anything Mesa reports: by the time Mesa
//! would tell us, we would have had to make the choice already.

use std::path::Path;

/// What we believe about the graphics stack, and why.
struct Verdict {
    backend: &'static str,
    /// Set `GALLIUM_DRIVER` to this before Slint initialises, if it is not already set.
    gallium: Option<&'static str>,
    reason: &'static str,
}

/// Decide the renderer and export the environment Slint reads.
///
/// Must be called before the first Slint call — `App::new()` reads `SLINT_BACKEND` and never looks
/// again. An explicit `SLINT_BACKEND` from the environment always wins, so a launch script or a
/// developer debugging a rendering problem can still force either renderer.
pub fn select() -> Renderer {
    if let Ok(existing) = std::env::var("SLINT_BACKEND") {
        tracing::info!(backend = %existing, "Renderer set explicitly; leaving it alone");
        return Renderer::from_backend(&existing);
    }

    let v = decide();

    if let Some(driver) = v.gallium {
        // Only fill this in if the operator has not expressed an opinion.
        if std::env::var("GALLIUM_DRIVER").is_err() {
            std::env::set_var("GALLIUM_DRIVER", driver);
            tracing::info!(driver, "Set GALLIUM_DRIVER so Mesa finds the GPU");
        }
    }

    std::env::set_var("SLINT_BACKEND", v.backend);
    tracing::info!(backend = v.backend, reason = v.reason, "Renderer selected");
    Renderer::from_backend(v.backend)
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

/// The decision itself, kept free of side effects so it can be reasoned about and tested.
fn decide() -> Verdict {
    // WSL2: the GPU is behind /dev/dxg, and Mesa reaches it through the d3d12 Gallium driver only
    // when told to. The presence of the node plus the driver is sufficient evidence.
    if Path::new("/dev/dxg").exists() && has_gallium_driver("d3d12") {
        return Verdict {
            backend: "winit-femtovg",
            gallium: Some("d3d12"),
            reason: "WSL /dev/dxg present with the d3d12 Mesa driver",
        };
    }

    // Native Linux: a render node means a DRM driver that can actually accept command buffers.
    // llvmpipe on its own does not create one, which is exactly the distinction that matters here.
    if has_render_node() {
        return Verdict {
            backend: "winit-femtovg",
            gallium: None,
            reason: "DRM render node present",
        };
    }

    Verdict {
        backend: "winit-software",
        gallium: None,
        reason: "no GPU found — the software rasteriser is far cheaper than software OpenGL",
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

/// True if any DRM render node exists (`/dev/dri/renderD*`).
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

    #[test]
    fn a_machine_with_no_gpu_gets_the_software_rasteriser() {
        // The decision on this builder reflects whatever hardware it has, but the invariant holds
        // either way: femtovg is never chosen without a positive hardware finding.
        let v = decide();
        if v.backend == "winit-femtovg" {
            assert!(
                Path::new("/dev/dxg").exists() || has_render_node(),
                "femtovg was chosen with no GPU evidence — this is the 576% CPU case"
            );
        } else {
            assert_eq!(v.backend, "winit-software");
        }
        assert!(!v.reason.is_empty(), "every verdict explains itself in the log");
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
        assert_eq!(Renderer::Gpu.ambient_interval_ms(), 16, "60fps where frames are free");
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
}
