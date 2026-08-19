//! Drosophila circadian activity: morning and evening peaks, a midday siesta,
//! night quiescence. Returns a multiplier for the sim's baseline drive.

/// Control points of the daily activity curve, linearly interpolated.
const POINTS: [(f32, f32); 10] = [
    (0.0, 0.25),
    (5.0, 0.25),
    (8.0, 1.0),
    (10.0, 1.0),
    (13.0, 0.55),
    (15.0, 0.55),
    (17.0, 1.0),
    (20.0, 1.0),
    (23.0, 0.3),
    (24.0, 0.25),
];

/// Activity multiplier for a local hour in `0.0..24.0`.
pub fn activity(hour: f32) -> f32 {
    for w in POINTS.windows(2) {
        let (h0, v0) = w[0];
        let (h1, v1) = w[1];
        if hour >= h0 && hour <= h1 {
            let t = (hour - h0) / (h1 - h0).max(0.001);
            return v0 + (v1 - v0) * t;
        }
    }
    0.25
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the original's behaviour suite checks: night and siesta dips,
    /// dawn and dusk peaks.
    #[test]
    fn the_curve_has_two_peaks_and_two_dips() {
        let night = activity(3.0);
        let dawn = activity(9.0);
        let siesta = activity(14.0);
        let dusk = activity(18.0);

        assert!(night < 0.4, "night {night}");
        assert!(dawn > 0.9, "dawn {dawn}");
        assert!((0.3..0.7).contains(&siesta), "siesta {siesta}");
        assert!(dusk > 0.9, "dusk {dusk}");
    }

    #[test]
    fn is_defined_across_the_whole_day() {
        for h in 0..=240 {
            let v = activity(h as f32 / 10.0);
            assert!(
                (0.2..=1.0).contains(&v),
                "hour {} gave {v}",
                h as f32 / 10.0
            );
        }
    }

    #[test]
    fn out_of_range_hours_fall_back_to_the_night_floor() {
        assert_eq!(activity(-1.0), 0.25);
        assert_eq!(activity(25.0), 0.25);
    }
}
