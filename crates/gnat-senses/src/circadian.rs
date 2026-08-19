//! Wall-clock rhythm. Ports from the original for free: no OS API involved.

use std::time::{SystemTime, UNIX_EPOCH};

/// Fraction of the way through the local day, 0.0 at midnight.
pub fn day_phase(now: SystemTime, utc_offset_secs: i32) -> f32 {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let local = secs + utc_offset_secs as i64;
    let of_day = local.rem_euclid(86_400) as f32;
    of_day / 86_400.0
}

/// Drosophila is crepuscular: two activity peaks, around dawn and dusk, with a
/// deep midday siesta and a deeper night. Returns 0.0..=1.0.
pub fn activity_drive(phase: f32) -> f32 {
    let peak = |centre: f32, width: f32| {
        let mut d = (phase - centre).abs();
        if d > 0.5 {
            d = 1.0 - d; // the day wraps
        }
        (-(d * d) / (2.0 * width * width)).exp()
    };
    // Dawn near 06:00, dusk near 19:00.
    let dawn = peak(6.0 / 24.0, 0.045);
    let dusk = peak(19.0 / 24.0, 0.055);
    let baseline = if (6.0 / 24.0..19.0 / 24.0).contains(&phase) {
        0.25
    } else {
        0.05
    };
    (dawn.max(dusk).max(baseline)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(hour: f32) -> f32 {
        day_phase(UNIX_EPOCH + Duration::from_secs_f32(hour * 3600.0), 0)
    }

    #[test]
    fn phase_wraps_the_day() {
        assert!((at(0.0) - 0.0).abs() < 1e-6);
        assert!((at(12.0) - 0.5).abs() < 1e-4);
        assert!((at(24.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn crepuscular_peaks_beat_midday_and_night() {
        let dawn = activity_drive(6.0 / 24.0);
        let dusk = activity_drive(19.0 / 24.0);
        let midday = activity_drive(13.0 / 24.0);
        let night = activity_drive(3.0 / 24.0);

        assert!(dawn > midday, "dawn {dawn} should beat midday {midday}");
        assert!(dusk > midday, "dusk {dusk} should beat midday {midday}");
        assert!(midday > night, "midday {midday} should beat night {night}");
    }
}
