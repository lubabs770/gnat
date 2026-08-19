//! The two coordinate frames, and how to get between them.
//!
//! * **Screen** — what Hyprland reports and what the overlay canvas is indexed
//!   by. Origin at the top-left of the output, +y **down**.
//! * **Scene** — what the body model works in, inherited from the original.
//!   Origin at the centre of the output, +y **up**.
//!
//! Keeping the conversion in one named place, rather than folding it into
//! either crate, is deliberate: a sign error here looks exactly like a
//! behaviour bug, and this is the only file where the sign can be wrong.

use gnat_body::Ledge as SceneLedge;
use gnat_senses::Ledge as ScreenLedge;

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub width: f32,
    pub height: f32,
}

impl Frame {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width as f32,
            height: height as f32,
        }
    }

    /// Output size, as the body model wants it.
    pub fn bounds(&self) -> (f32, f32) {
        (self.width, self.height)
    }

    pub fn to_scene(self, x: f32, y: f32) -> (f32, f32) {
        (x - self.width / 2.0, self.height / 2.0 - y)
    }

    pub fn to_screen(self, x: f32, y: f32) -> (f32, f32) {
        (x + self.width / 2.0, self.height / 2.0 - y)
    }

    /// Convert walkable edges into the body's frame, keeping window identity so
    /// the fly notices when the thing under its feet closes.
    pub fn ledges_to_scene(self, ledges: &[ScreenLedge]) -> Vec<SceneLedge> {
        ledges
            .iter()
            .map(|l| SceneLedge {
                y: self.height / 2.0 - l.y as f32,
                x0: l.x0 as f32 - self.width / 2.0,
                x1: l.x1 as f32 - self.width / 2.0,
                id: l.id,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::new(1920, 1080)
    }

    #[test]
    fn the_centre_of_the_screen_is_the_origin() {
        assert_eq!(frame().to_scene(960.0, 540.0), (0.0, 0.0));
        assert_eq!(frame().to_screen(0.0, 0.0), (960.0, 540.0));
    }

    #[test]
    fn y_flips_between_the_frames() {
        // The top of the screen is the top of the scene, which is +y there.
        let (_, top) = frame().to_scene(0.0, 0.0);
        let (_, bottom) = frame().to_scene(0.0, 1080.0);
        assert_eq!(top, 540.0);
        assert_eq!(bottom, -540.0);
        assert!(top > bottom, "scene +y must point up");
    }

    #[test]
    fn conversion_round_trips() {
        let f = frame();
        for (x, y) in [(0.0, 0.0), (1919.0, 1079.0), (37.0, 811.0)] {
            let (sx, sy) = f.to_scene(x, y);
            let (bx, by) = f.to_screen(sx, sy);
            assert!((bx - x).abs() < 1e-3 && (by - y).abs() < 1e-3, "{x},{y}");
        }
    }

    #[test]
    fn ledges_keep_their_window_identity_and_flip_y() {
        let f = frame();
        let screen = [ScreenLedge {
            y: 200,
            x0: 100,
            x1: 500,
            id: 0xABC,
        }];
        let scene = f.ledges_to_scene(&screen);
        assert_eq!(scene.len(), 1);
        assert_eq!(scene[0].id, 0xABC);
        assert_eq!(scene[0].y, 340.0); // 540 - 200
        assert_eq!(scene[0].x0, -860.0);
        assert_eq!(scene[0].x1, -460.0);
    }

    #[test]
    fn a_window_higher_on_screen_is_higher_in_the_scene() {
        let f = frame();
        let high = f.ledges_to_scene(&[ScreenLedge {
            y: 100,
            x0: 0,
            x1: 10,
            id: 1,
        }]);
        let low = f.ledges_to_scene(&[ScreenLedge {
            y: 900,
            x0: 0,
            x1: 10,
            id: 2,
        }]);
        assert!(
            high[0].y > low[0].y,
            "a ledge near the top of the screen must sit higher in the scene"
        );
    }
}
