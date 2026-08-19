//! Wall clock. Ports from the original for free: no OS API involved.
//!
//! Only the clock lives here. The *shape* of the fly's day — dawn and dusk
//! peaks, midday siesta, night quiescence — belongs to the body model, in
//! `gnat_body::circadian`, because it is biology rather than a desktop sense.

use std::time::{SystemTime, UNIX_EPOCH};

/// Local hour of the day as a fraction, `0.0..24.0`.
pub fn local_hour(now: SystemTime, utc_offset_secs: i32) -> f32 {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let local = secs + utc_offset_secs as i64;
    local.rem_euclid(86_400) as f32 / 3600.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(hour: f32) -> f32 {
        local_hour(UNIX_EPOCH + Duration::from_secs_f32(hour * 3600.0), 0)
    }

    #[test]
    fn hour_wraps_the_day() {
        assert!((at(0.0) - 0.0).abs() < 1e-4);
        assert!((at(12.5) - 12.5).abs() < 1e-2);
        assert!((at(24.0) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn the_utc_offset_shifts_the_clock() {
        let utc = local_hour(UNIX_EPOCH + Duration::from_secs(0), 0);
        let plus_two = local_hour(UNIX_EPOCH + Duration::from_secs(0), 2 * 3600);
        assert!((plus_two - utc - 2.0).abs() < 1e-4);
    }
}
