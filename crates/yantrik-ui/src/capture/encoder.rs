//! Which encoder a recording uses.
//!
//! H.264 through VA-API when the GPU's render node can encode it, because then the CPU does
//! nearly nothing and the person's work is not slowed by being recorded. Otherwise `libx264` at
//! `veryfast`, CRF 23: the preset a laptop CPU keeps up with at 30 fps on a 1080p desktop, at a
//! quality nobody squints at. "Can encode" is asked, not assumed from the node existing: a VM's
//! virtio GPU has a render node and no encoder, and wf-recorder told to use one there exits at
//! once with nothing written. `vainfo` is the asking; with no answer, the CPU path is taken,
//! because the wrong guess the other way is a recording that never starts.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The first render node, which is the GPU on every machine this image targets.
pub const RENDER_NODE: &str = "/dev/dri/renderD128";

/// Frames per second. 30 is smooth for a desktop and half the size of 60.
pub const FPS: u32 = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Encoder {
    /// H.264 on the GPU, through this render node.
    Vaapi(PathBuf),
    /// H.264 on the CPU.
    X264,
}

impl Encoder {
    /// wf-recorder's arguments for this encoder and the frame rate.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = match self {
            // The filter is the one wf-recorder's manual gives for VA-API: frames arrive as RGB
            // and the encoder takes NV12.
            Encoder::Vaapi(node) => vec![
                "-c".into(),
                "h264_vaapi".into(),
                "-d".into(),
                node.to_string_lossy().into_owned(),
                "-F".into(),
                "scale_vaapi=format=nv12:out_range=full".into(),
            ],
            // yuv420p, because a 4:4:4 H.264 file is one most players (and every browser)
            // refuse to play.
            Encoder::X264 => vec![
                "-c".into(),
                "libx264".into(),
                "-p".into(),
                "preset=veryfast".into(),
                "-p".into(),
                "crf=23".into(),
                "-x".into(),
                "yuv420p".into(),
            ],
        };
        args.extend(["-r".to_string(), FPS.to_string()]);
        args
    }

    pub fn name(&self) -> &'static str {
        match self {
            Encoder::Vaapi(_) => "h264_vaapi",
            Encoder::X264 => "libx264",
        }
    }
}

/// Whether `vainfo`'s report says this device encodes H.264: an H.264 profile with an encode
/// entrypoint (`VAEntrypointEncSlice`, or the low-power `EncSliceLP` Intel exposes).
pub fn encodes_h264(vainfo: &str) -> bool {
    vainfo.lines().any(|line| line.contains("VAProfileH264") && line.contains("VAEntrypointEncSlice"))
}

/// The choice, from what was found: the node, and what `vainfo` said about it, if anything.
pub fn choose(node: &Path, node_exists: bool, vainfo: Option<&str>) -> Encoder {
    match vainfo {
        Some(report) if node_exists && encodes_h264(report) => Encoder::Vaapi(node.to_path_buf()),
        _ => Encoder::X264,
    }
}

/// Ask this machine. Blocking (vainfo opens the driver: well under a second).
pub fn probe() -> Encoder {
    let node = Path::new(RENDER_NODE);
    let exists = node.exists();
    let report = if exists {
        let mut cmd = Command::new("vainfo");
        cmd.args(["--display", "drm", "--device", RENDER_NODE]);
        crate::capture::grab::run_bounded(cmd, Duration::from_secs(5), "vainfo")
            .ok()
            .map(|out| String::from_utf8_lossy(&out).into_owned())
    } else {
        None
    };
    choose(node, exists, report.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTEL: &str = "vainfo: Driver version: Intel iHD driver\n\
        VAProfileH264Main               :\tVAEntrypointVLD\n\
        VAProfileH264Main               :\tVAEntrypointEncSliceLP\n";
    const DECODE_ONLY: &str = "VAProfileH264Main               :\tVAEntrypointVLD\n\
        VAProfileHEVCMain               :\tVAEntrypointEncSlice\n";

    #[test]
    fn the_gpu_encodes_when_its_render_node_says_it_can() {
        let node = Path::new(RENDER_NODE);
        assert_eq!(choose(node, true, Some(INTEL)), Encoder::Vaapi(node.to_path_buf()));
        let args = choose(node, true, Some(INTEL)).args();
        assert!(args.windows(2).any(|w| w == ["-c", "h264_vaapi"]), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["-d", RENDER_NODE]), "{args:?}");
    }

    #[test]
    fn otherwise_the_cpu_encodes_at_veryfast_crf_23() {
        let node = Path::new(RENDER_NODE);
        // No node; a node that only decodes H.264 (HEVC encode does not count); no vainfo at all.
        for (exists, report) in [(false, Some(INTEL)), (true, Some(DECODE_ONLY)), (true, None)] {
            assert_eq!(choose(node, exists, report), Encoder::X264, "{exists} {report:?}");
        }
        let args = Encoder::X264.args();
        for pair in [["-c", "libx264"], ["-p", "preset=veryfast"], ["-p", "crf=23"], ["-r", "30"]] {
            assert!(args.windows(2).any(|w| w == pair), "{pair:?} in {args:?}");
        }
    }
}
