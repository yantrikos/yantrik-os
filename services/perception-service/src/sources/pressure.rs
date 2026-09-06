//! What the machine feels, rather than what it is doing.
//!
//! A CPU percentage says how busy the machine is. It does not say whether anything is *waiting*,
//! and waiting is the only part a person notices. A box at 100% CPU with everything it needs
//! resident feels fast; a box at 30% CPU thrashing its page cache feels broken.
//!
//! PSI answers the second question directly: `some avg10` is the fraction of the last ten seconds
//! in which at least one task was stalled on a resource. It is the closest thing the kernel has to
//! "the computer feels slow", which is what someone would actually say.
//!
//! Reported on a crossing, not on a timer. Pressure that has not changed is not news, and a
//! stream that repeats itself teaches whatever reads it to ignore it.

use std::collections::HashMap;
use std::time::Duration;

use crate::bus::Bus;
use crate::observation::Kind;

/// Bands, not a threshold. A single trip point at 20% would emit on every wobble across it; three
/// bands mean a report happens when the *character* of the pressure changes.
const BANDS: [f32; 3] = [20.0, 50.0, 80.0];

const RESOURCES: [&str; 3] = ["cpu", "memory", "io"];

const INTERVAL: Duration = Duration::from_secs(5);

pub fn run(bus: Bus) {
    if !std::path::Path::new("/proc/pressure/cpu").exists() {
        bus.push(
            Kind::SourceFailed {
                source: "pressure".into(),
                reason: "/proc/pressure is absent — kernel built without PSI".into(),
            },
            None,
        );
        tracing::warn!("PSI unavailable; no pressure observations");
        return;
    }
    tracing::info!("Pressure watching /proc/pressure");

    let mut last_band: HashMap<&str, usize> = HashMap::new();
    loop {
        for resource in RESOURCES {
            let Some(stalled) = read_some_avg10(resource) else { continue };
            let band = band_of(stalled);
            let previous = last_band.get(resource).copied().unwrap_or(0);
            if band != previous {
                last_band.insert(resource, band);
                // Including the fall back to nothing: "it is over" is as useful as "it started",
                // and a stream that only ever reports trouble never reports relief.
                bus.push(
                    Kind::Pressure { resource: resource.to_string(), stalled_pct_10s: stalled },
                    None,
                );
            }
        }
        std::thread::sleep(INTERVAL);
    }
}

/// `some avg10=12.34 avg60=... ` — the first number of the first line.
fn read_some_avg10(resource: &str) -> Option<f32> {
    let text = std::fs::read_to_string(format!("/proc/pressure/{resource}")).ok()?;
    parse_some_avg10(&text)
}

fn parse_some_avg10(text: &str) -> Option<f32> {
    text.lines()
        .find(|l| l.starts_with("some "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))
        .and_then(|v| v.parse().ok())
}

fn band_of(stalled: f32) -> usize {
    BANDS.iter().filter(|edge| stalled >= **edge).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "some avg10=12.34 avg60=5.00 avg300=1.00 total=123456\n\
                          full avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

    #[test]
    fn it_reads_the_some_line_not_the_full_one() {
        // `full` is every task stalled at once, which on a desktop is almost never true. `some`
        // is the one that corresponds to the machine feeling slow.
        assert_eq!(parse_some_avg10(SAMPLE), Some(12.34));
    }

    #[test]
    fn a_kernel_without_psi_yields_nothing_rather_than_zero() {
        // Zero would read as "the machine is fine", which is a claim we would not be entitled to.
        assert_eq!(parse_some_avg10(""), None);
        assert_eq!(parse_some_avg10("total=0\n"), None);
    }

    #[test]
    fn bands_change_on_character_not_on_wobble() {
        assert_eq!(band_of(0.0), 0);
        assert_eq!(band_of(19.9), 0);
        assert_eq!(band_of(20.0), 1);
        assert_eq!(band_of(49.9), 1);
        assert_eq!(band_of(55.0), 2);
        assert_eq!(band_of(95.0), 3);
    }

    #[test]
    fn the_real_file_parses_where_it_exists() {
        if let Ok(text) = std::fs::read_to_string("/proc/pressure/cpu") {
            assert!(
                parse_some_avg10(&text).is_some(),
                "this kernel has PSI but we could not read it: {text:?}"
            );
        }
    }
}
