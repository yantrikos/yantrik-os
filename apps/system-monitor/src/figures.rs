//! What the window draws of the model, at the precision it draws it.
//!
//! A percentage is drawn as a whole number (`Math.round` in `system_monitor.slint`), and a
//! process's share to a tenth, but the reading holds them to a hundredth and beyond. Set as they
//! were read, a CPU going from 37.21% to 37.24% was a changed property and a frame, with nothing
//! on screen different; on the software renderer of a machine with no GPU every frame is the
//! whole window. So the figures are rounded here to what is drawn, once, and `view` sets these:
//! a reading that moves nothing visible draws nothing. A bar's fill is drawn from the same
//! whole percentage as the number beside it, which moves it by less than half a percent of its
//! width.
//!
//! Comparing two [`Figures`] is also how a reading is known to have changed the screen, which is
//! what `crate::update` owes a frame for (`yantrik_app::shown`). `describe` reads the model, not
//! these, so the Mind's figures keep their tenths.

use slint::SharedString;

use crate::model::Model;
use crate::{CpuCoreData, DiskData, MonitorProcessData, NetworkInterfaceData};

/// Everything the cards, the status row and the process list draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Figures {
    pub cpu_measured: bool,
    pub cpu_usage: f32,
    pub load_avg: [SharedString; 3],
    pub cores: Vec<CpuCoreData>,
    pub memory_usage: f32,
    pub memory_used: SharedString,
    pub memory_total: SharedString,
    pub memory_available: SharedString,
    pub memory_cached: SharedString,
    pub memory_buffers: SharedString,
    pub swap_usage: f32,
    pub swap_used: SharedString,
    pub swap_total: SharedString,
    pub disks: Vec<DiskData>,
    pub interfaces: Vec<NetworkInterfaceData>,
    pub uptime: SharedString,
    pub status_lead: SharedString,
    pub status_rest: SharedString,
    pub status_thresholds: SharedString,
    pub status_needs_you: bool,
    /// The list through the search box, in the order asked for.
    pub processes: Vec<MonitorProcessData>,
}

/// A percentage as the cards draw it: a whole number.
pub fn whole(percent: f32) -> f32 {
    percent.round()
}

/// A process's share as its row draws it: to a tenth.
pub fn tenth(percent: f64) -> f32 {
    ((percent * 10.0).round() / 10.0) as f32
}

impl Figures {
    pub fn of(m: &Model) -> Figures {
        let r = &m.readout;
        let text = |s: &str| SharedString::from(s);
        Figures {
            cpu_measured: r.cpu_measured,
            cpu_usage: whole(r.cpu_usage),
            load_avg: [text(&r.load_avg[0]), text(&r.load_avg[1]), text(&r.load_avg[2])],
            cores: r
                .cores
                .iter()
                .map(|c| CpuCoreData { core_id: c.id, usage: whole(c.usage) })
                .collect(),
            memory_usage: whole(r.memory_usage),
            memory_used: text(&r.memory_used),
            memory_total: text(&r.memory_total),
            memory_available: text(&r.memory_available),
            memory_cached: text(&r.memory_cached),
            memory_buffers: text(&r.memory_buffers),
            swap_usage: whole(r.swap_usage),
            swap_used: text(&r.swap_used),
            swap_total: text(&r.swap_total),
            disks: r
                .disks
                .iter()
                .map(|d| DiskData {
                    mount_point: text(&d.mount_point),
                    filesystem: text(&d.filesystem),
                    used_bytes: text(&d.used),
                    total_bytes: text(&d.total),
                    usage_percent: whole(d.usage_percent),
                    needs_you: d.needs_you,
                })
                .collect(),
            interfaces: r
                .interfaces
                .iter()
                .map(|n| NetworkInterfaceData {
                    name: text(&n.name),
                    ip_address: SharedString::new(),
                    rx_bytes: text(&n.rx_bytes),
                    tx_bytes: text(&n.tx_bytes),
                    rx_speed: text(&n.rx_speed),
                    tx_speed: text(&n.tx_speed),
                })
                .collect(),
            uptime: text(&r.uptime),
            status_lead: text(&m.status.lead),
            status_rest: text(&m.status.rest),
            status_thresholds: text(&m.status.thresholds),
            status_needs_you: m.status.needs_you(),
            processes: m
                .shown()
                .iter()
                .map(|p| MonitorProcessData {
                    pid: p.pid as i32,
                    name: text(&p.name),
                    cpu_percent: tenth(p.cpu_percent),
                    mem_percent: tenth(p.mem_percent),
                    status: text(&p.state),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_figure_is_held_to_what_is_drawn() {
        assert_eq!(whole(37.21), 37.0);
        assert_eq!(whole(37.24), whole(37.21), "a move nobody can see is no change");
        assert_eq!(whole(37.6), 38.0);
        assert_eq!(tenth(30.04), 30.0);
        assert_eq!(tenth(30.06), 30.1);
    }
}
