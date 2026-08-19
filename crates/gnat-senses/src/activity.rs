//! "Is the human there?" — the vibration sense.
//!
//! The original used the macOS idle-time API, which reports *that* input
//! happened and never *what* was typed. Wayland's equivalent is
//! `ext-idle-notify-v1`, and Hyprland implements it; wiring that up needs a
//! Wayland connection, so it lands with the overlay crate.
//!
//! Until then this derives the same signal without any extra protocol: the
//! cursor moving, or Hyprland emitting any event at all, both mean a human is
//! at the keyboard. It is content-blind by construction — we never see a
//! keycode, only that something changed.

use std::time::{Duration, Instant};

pub struct Activity {
    last_input: Instant,
    idle_after: Duration,
    /// Rolling estimate of how busy the desktop is, 0.0..=1.0.
    intensity: f32,
}

impl Activity {
    pub fn new(idle_after: Duration) -> Self {
        Self {
            last_input: Instant::now(),
            idle_after,
            intensity: 0.0,
        }
    }

    /// Call whenever any evidence of a human arrives: cursor movement, a
    /// Hyprland event, a focus change.
    pub fn poke(&mut self) {
        self.last_input = Instant::now();
        self.intensity = (self.intensity + 0.25).min(1.0);
    }

    /// Call once per frame to let the estimate decay.
    pub fn tick(&mut self, dt: Duration) {
        let decay = (-dt.as_secs_f32() / 2.0).exp();
        self.intensity *= decay;
    }

    pub fn idle_for(&self) -> Duration {
        self.last_input.elapsed()
    }

    pub fn is_idle(&self) -> bool {
        self.idle_for() >= self.idle_after
    }

    /// Substrate vibration the fly can feel, 0.0..=1.0.
    pub fn vibration(&self) -> f32 {
        self.intensity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poking_raises_vibration_and_decay_lowers_it() {
        let mut a = Activity::new(Duration::from_secs(60));
        assert_eq!(a.vibration(), 0.0);
        a.poke();
        a.poke();
        let hot = a.vibration();
        assert!(hot > 0.0);
        a.tick(Duration::from_secs(10));
        assert!(a.vibration() < hot);
    }

    #[test]
    fn fresh_activity_is_not_idle() {
        let a = Activity::new(Duration::from_secs(60));
        assert!(!a.is_idle());
    }
}
