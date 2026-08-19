//! Drawing the fly into the overlay's software canvas.
//!
//! Top-down, because that is the view a desktop gives you. The body model is
//! the source of every angle here — this file invents nothing about behaviour,
//! it only decides what the numbers look like.
//!
//! The original is SceneKit and gets lighting and depth for free. This is a
//! plain pixel buffer, so altitude is conveyed the way an animator would: the
//! fly scales up, and its shadow slides away and softens.

use gnat_body::Fly;
use gnat_overlay::{Canvas, Rgba};

use crate::coord::Frame;

const THORAX: Rgba = Rgba::opaque(96, 72, 42);
const ABDOMEN: Rgba = Rgba::opaque(146, 110, 62);
const BAND: Rgba = Rgba::new(48, 34, 20, 130);
const HEAD: Rgba = Rgba::opaque(104, 78, 44);
const EYE: Rgba = Rgba::opaque(158, 44, 36);
const LEG: Rgba = Rgba::opaque(70, 51, 30);
const WING: Rgba = Rgba::new(214, 226, 240, 70);
const WING_VEIN: Rgba = Rgba::new(150, 168, 190, 60);

// Body-local landmarks, in scene units. +y is forward, +x is right.
const HEAD_Y: f32 = 7.0;
const THORAX_Y: f32 = 1.4;
const ABDOMEN_Y: f32 = -5.6;
/// Where each leg meets the body, from the original's attachment table.
const LEG_ATTACH: [(f32, f32); 6] = [
    (2.6, 4.6),
    (-2.6, 4.6),
    (3.2, 1.6),
    (-3.2, 1.6),
    (2.9, -1.0),
    (-2.9, -1.0),
];
/// Real fly legs are tucked close under the body from above. Longer than this
/// and it reads as a spider — which is exactly what the first version did.
const LEG_LEN: f32 = 7.2;

/// Maps body-local coordinates onto the canvas.
struct Pose {
    origin: (f32, f32),
    /// Unit vectors, in screen pixels.
    forward: (f32, f32),
    right: (f32, f32),
    scale: f32,
}

impl Pose {
    fn new(fly: &Fly, frame: &Frame) -> Self {
        let origin = frame.to_screen(fly.pos.0, fly.pos.1);
        let (sin, cos) = fly.heading.sin_cos();
        Self {
            origin,
            // Scene +y is up and screen +y is down, so every y component flips.
            forward: (cos, -sin),
            right: (sin, cos),
            scale: fly.scale,
        }
    }

    /// A body-local point, in canvas pixels.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (x * self.scale, y * self.scale);
        (
            self.origin.0 + self.forward.0 * y + self.right.0 * x,
            self.origin.1 + self.forward.1 * y + self.right.1 * x,
        )
    }

    /// Rotation to hand [`Canvas::ellipse`] so that its **ry** axis runs along
    /// the body.
    ///
    /// `ellipse` puts `rx` along the angle it is given and `ry` across it, so
    /// passing the heading directly draws every segment broadside-on. That is
    /// a quarter turn, and it is not subtle: it turns the fly into a blob.
    fn body_angle(&self, extra: f32) -> f32 {
        self.forward.1.atan2(self.forward.0) - std::f32::consts::FRAC_PI_2 + extra
    }

    fn px(&self, units: f32) -> f32 {
        units * self.scale
    }
}

/// Draw one fly.
pub fn draw_fly(canvas: &mut Canvas, fly: &Fly, frame: &Frame) {
    // The shadow belongs to the surface, so it goes down first.
    draw_shadow(canvas, fly, frame);

    let p = Pose::new(fly, frame);
    draw_legs(canvas, fly, &p);

    // Abdomen, then thorax, then head: back to front along the body.
    let (ax, ay) = p.at(0.0, ABDOMEN_Y);
    canvas.ellipse(
        ax,
        ay,
        p.px(3.3),
        p.px(5.6) * fly.breath,
        p.body_angle(0.0),
        ABDOMEN,
    );
    // Two dark tergite bands, the thing that makes a brown blob read as an
    // abdomen at this size.
    for k in [-0.35f32, 0.25] {
        let (bx, by) = p.at(0.0, ABDOMEN_Y + k * 5.6);
        canvas.ellipse(bx, by, p.px(3.2), p.px(0.7), p.body_angle(0.0), BAND);
    }

    // Wings sit over the abdomen: folded there at rest, sweeping through it in
    // flight. They are never hidden — a fly with no wings reads as an ant.
    draw_wings(canvas, fly, &p);

    let (tx, ty) = p.at(0.0, THORAX_Y);
    canvas.ellipse(tx, ty, p.px(3.4), p.px(4.4), p.body_angle(0.0), THORAX);

    let (hx, hy) = p.at(0.0, HEAD_Y);
    canvas.ellipse(hx, hy, p.px(3.0), p.px(2.4), p.body_angle(0.0), HEAD);
    // The eyes wrap the sides of the head and take up most of it, which is the
    // single strongest "this is a fly" cue at twenty pixels.
    for side in [-1.0f32, 1.0] {
        let (ex, ey) = p.at(side * 1.5, HEAD_Y + 0.2);
        canvas.ellipse(ex, ey, p.px(1.8), p.px(2.1), p.body_angle(side * 0.35), EYE);
    }

    for side in [-1.0f32, 1.0] {
        let (bx, by) = p.at(side * 0.8, HEAD_Y + 1.2);
        let (tipx, tipy) = p.at(side * 1.8, HEAD_Y + 2.8);
        canvas.line(bx, by, tipx, tipy, p.px(0.5), LEG);
    }
}

fn draw_legs(canvas: &mut Canvas, fly: &Fly, p: &Pose) {
    for (leg, &(ax, ay)) in fly.legs.iter().zip(LEG_ATTACH.iter()) {
        // `lift` raises the leg mid-swing; from directly above that reads as
        // foreshortening rather than height.
        let reach = LEG_LEN * (1.0 - 0.30 * leg.lift);
        let dir = leg.base_yaw + leg.swing_sign * leg.angle;
        let (root_x, root_y) = p.at(ax, ay);

        let knee_local = (ax + dir.cos() * reach * 0.5, ay + dir.sin() * reach * 0.5);
        // The lower segment folds back, so a leg reads as a joint and not a
        // spoke radiating from the body.
        let fold = dir + leg.swing_sign * 0.9;
        let foot_local = (
            knee_local.0 + fold.cos() * reach * 0.5,
            knee_local.1 + fold.sin() * reach * 0.5,
        );
        let knee = p.at(knee_local.0, knee_local.1);
        let foot = p.at(foot_local.0, foot_local.1);

        canvas.line(root_x, root_y, knee.0, knee.1, p.px(0.8), LEG);
        canvas.line(knee.0, knee.1, foot.0, foot.1, p.px(0.6), LEG);
    }
}

fn draw_shadow(canvas: &mut Canvas, fly: &Fly, frame: &Frame) {
    // Height pushes the shadow away and fades it, which is the only altitude
    // cue a flat canvas has.
    let lift = (fly.z / 90.0).clamp(0.0, 1.0);
    let (sx, sy) = frame.to_screen(fly.pos.0 - 10.0 * lift, fly.pos.1 - 16.0 * lift);
    let alpha = (75.0 * (1.0 - 0.7 * lift)) as u8;
    let spread = 1.0 + 0.6 * lift;
    let (sin, cos) = fly.heading.sin_cos();
    let angle = (-sin).atan2(cos) - std::f32::consts::FRAC_PI_2;
    canvas.ellipse(
        sx,
        sy,
        4.0 * fly.scale * spread,
        8.5 * fly.scale * spread,
        angle,
        Rgba::new(0, 0, 0, alpha),
    );
}

fn draw_wings(canvas: &mut Canvas, fly: &Fly, p: &Pose) {
    const HALF_LEN: f32 = 6.4;
    for (i, wing) in fly.wings.iter().enumerate() {
        let side = if i == 0 { -1.0f32 } else { 1.0 };
        // `wing.z` is how far the wing is swept out from the body axis; at rest
        // it is near 0.13 and folded flat, in flight it sweeps to about 0.8.
        let sweep = 0.30 + wing.z.abs();
        // `wing.x` is the stroke; seen from above it mostly shortens the wing.
        let foreshorten = 1.0 - 0.40 * wing.x.abs();
        let len = HALF_LEN * foreshorten;

        // Hinge at the rear of the thorax, wing running back and outward.
        let hinge = (side * 1.4, THORAX_Y - 1.6);
        let centre_local = (
            hinge.0 + side * len * sweep.sin(),
            hinge.1 - len * sweep.cos(),
        );
        let (cx, cy) = p.at(centre_local.0, centre_local.1);
        let angle = p.body_angle(-side * sweep);

        canvas.ellipse(cx, cy, p.px(2.0), p.px(len), angle, WING);
        // One vein down the middle: enough to stop the wing reading as a smear.
        canvas.ellipse(cx, cy, p.px(0.4), p.px(len * 0.85), angle, WING_VEIN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnat_body::State;

    const W: u32 = 200;
    const H: u32 = 200;

    /// Draw one fly into a blank canvas and return the buffer.
    fn render(configure: impl FnOnce(&mut Fly)) -> Vec<u8> {
        let mut fly = Fly::new((0.0, 0.0), 7);
        fly.heading = 0.0;
        configure(&mut fly);

        let frame = Frame::new(W, H);
        let mut pixels = vec![0u8; (W * H * 4) as usize];
        let mut canvas = Canvas {
            width: W,
            height: H,
            pixels: &mut pixels,
            time_ms: 0,
        };
        draw_fly(&mut canvas, &fly, &frame);
        pixels
    }

    fn alpha_at(px: &[u8], x: u32, y: u32) -> u8 {
        px[((y * W + x) * 4 + 3) as usize]
    }

    fn painted(px: &[u8]) -> usize {
        px.chunks_exact(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn a_fly_at_the_origin_paints_the_middle_and_nothing_else() {
        let px = render(|_| {});
        assert!(alpha_at(&px, W / 2, H / 2) > 0, "nothing drawn at the fly");
        for (x, y) in [(0, 0), (W - 1, 0), (0, H - 1), (W - 1, H - 1)] {
            assert_eq!(alpha_at(&px, x, y), 0, "painted the corner at {x},{y}");
        }
        // A fly is small. Covering a large share of a 200x200 canvas means the
        // geometry has blown up.
        let share = painted(&px) as f32 / (W * H) as f32;
        assert!(
            (0.005..0.15).contains(&share),
            "fly covers {:.1}% of the canvas",
            share * 100.0
        );
    }

    #[test]
    fn rotating_the_fly_changes_what_is_drawn() {
        let flat = render(|f| f.heading = 0.0);
        let turned = render(|f| f.heading = std::f32::consts::FRAC_PI_2);
        assert_ne!(flat, turned, "heading had no effect on the render");
    }

    #[test]
    fn altitude_makes_the_fly_bigger() {
        let low = painted(&render(|f| f.scale = 1.15));
        let high = painted(&render(|f| f.scale = 2.0));
        assert!(high > low, "a higher fly should cover more pixels");
    }

    #[test]
    fn wings_are_drawn_on_the_ground_too() {
        // Upstream keeps the folded pair visible and only hides the motion
        // blur; a wingless fly reads as an ant.
        let grounded = painted(&render(|f| f.state = State::Idle));
        let no_wings = painted(&render(|f| {
            f.state = State::Idle;
            f.wings = [Default::default(); 2];
        }));
        assert!(grounded > no_wings, "folded wings were not drawn");
    }

    #[test]
    fn a_fly_off_the_canvas_draws_nothing_and_does_not_panic() {
        let px = render(|f| f.pos = (10_000.0, 10_000.0));
        assert_eq!(painted(&px), 0);
    }
}
