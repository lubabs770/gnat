//! Turning window geometry into a world the fly can walk on, and turning
//! cursor motion into a looming stimulus.
//!
//! This is the sense-mapping layer: everything above it is raw desktop state,
//! everything below it is sim input.

use crate::hypr::{Client, CursorPos};

/// A walkable horizontal edge in screen coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ledge {
    pub y: i32,
    pub x0: i32,
    pub x1: i32,
    /// The window this edge belongs to, so the fly can notice when the thing
    /// it is standing on closes. Zero is the screen floor.
    pub id: u64,
}

impl Ledge {
    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }
    pub fn supports(&self, x: i32) -> bool {
        x >= self.x0 && x < self.x1
    }
}

/// The set of window top edges the fly can stand on, plus the screen floor.
///
/// `clients` must already be front-to-back — [`crate::hypr::Hypr::clients`]
/// returns it that way. A ledge is only walkable where no window in front of
/// it covers that span, which is what stops the fly walking along an edge that
/// is visually buried.
pub fn ledges(clients: &[Client], screen_w: i32, screen_h: i32) -> Vec<Ledge> {
    let mut out = vec![Ledge {
        y: screen_h,
        x0: 0,
        x1: screen_w,
        id: 0,
    }];

    for (i, c) in clients.iter().enumerate() {
        if !c.is_terrain() {
            continue;
        }
        // Occluders are the windows stacked in front of this one.
        let occluders: Vec<&Client> = clients[..i]
            .iter()
            .filter(|o| o.is_terrain() && o.top() <= c.top() && o.bottom() > c.top())
            .collect();

        for (x0, x1) in subtract_spans(c.left(), c.right(), &occluders) {
            if x1 > x0 {
                out.push(Ledge {
                    y: c.top(),
                    x0,
                    x1,
                    id: c.id(),
                });
            }
        }
    }
    out
}

/// Remove each occluder's horizontal span from `[x0, x1)`, returning what is
/// left, in order.
fn subtract_spans(x0: i32, x1: i32, occluders: &[&Client]) -> Vec<(i32, i32)> {
    let mut spans = vec![(x0, x1)];
    for o in occluders {
        let (a, b) = (o.left(), o.right());
        spans = spans
            .into_iter()
            .flat_map(|(s, e)| {
                if b <= s || a >= e {
                    vec![(s, e)] // no overlap
                } else {
                    let mut parts = Vec::new();
                    if a > s {
                        parts.push((s, a));
                    }
                    if b < e {
                        parts.push((b, e));
                    }
                    parts
                }
            })
            .collect();
    }
    spans
}

/// The looming pathway's input: how threatening an approaching object is.
///
/// Biologically this is angular subtense expanding on the retina. Here the
/// object is the cursor (or a newly-opened window), and the drive rises with
/// closeness and with closing speed. Something large and near but *stationary*
/// is not a loom, which is why velocity is a factor and not a bonus.
pub fn loom_drive(fly: (f32, f32), obj: (f32, f32), obj_vel: (f32, f32), radius: f32) -> f32 {
    let dx = obj.0 - fly.0;
    let dy = obj.1 - fly.1;
    let dist = (dx * dx + dy * dy).sqrt().max(1.0);

    if dist > radius {
        return 0.0;
    }
    // Closing speed: the component of velocity pointing at the fly.
    let closing = -(obj_vel.0 * dx + obj_vel.1 * dy) / dist;
    if closing <= 0.0 {
        return 0.0;
    }
    // Angular expansion rate, the standard loom cue. Soft-saturated rather
    // than clamped: a bare clamp pins every fast approach to 1.0 and throws
    // away the near/far ordering the escape decision depends on.
    let expansion = closing / dist;
    let urgency = 1.0 - (-expansion / EXPANSION_SCALE).exp();
    let proximity = 1.0 - dist / radius;
    (urgency * proximity).clamp(0.0, 1.0)
}

/// Expansion rate, in reciprocal seconds, at which the loom response is
/// roughly two thirds of maximum.
const EXPANSION_SCALE: f32 = 5.0;

/// Tracks the cursor between polls so [`loom_drive`] has a velocity to use.
pub struct CursorTracker {
    last: Option<(CursorPos, std::time::Instant)>,
    vel: (f32, f32),
}

impl Default for CursorTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorTracker {
    pub fn new() -> Self {
        Self {
            last: None,
            vel: (0.0, 0.0),
        }
    }

    /// Feed a fresh poll. Returns the current position.
    pub fn update(&mut self, pos: CursorPos) -> CursorPos {
        let now = std::time::Instant::now();
        if let Some((prev, then)) = self.last {
            let dt = now.duration_since(then).as_secs_f32();
            if dt > 1e-4 {
                // Smoothed, so one jittery poll cannot fire an escape.
                let raw = ((pos.x - prev.x) as f32 / dt, (pos.y - prev.y) as f32 / dt);
                self.vel = (
                    self.vel.0 * 0.5 + raw.0 * 0.5,
                    self.vel.1 * 0.5 + raw.1 * 0.5,
                );
            }
        }
        self.last = Some((pos, now));
        pos
    }

    pub fn velocity(&self) -> (f32, f32) {
        self.vel
    }

    pub fn moved(&self, threshold: f32) -> bool {
        let (vx, vy) = self.vel;
        (vx * vx + vy * vy).sqrt() > threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(x: i32, y: i32, w: i32, h: i32) -> Client {
        Client {
            address: format!("{x},{y}"),
            at: (x, y),
            size: (w, h),
            class: "test".into(),
            title: "test".into(),
            floating: false,
            mapped: true,
            hidden: false,
            monitor: 0,
            focus_history_id: 0,
        }
    }

    #[test]
    fn the_floor_always_exists() {
        let l = ledges(&[], 1920, 1080);
        assert_eq!(
            l,
            vec![Ledge {
                y: 1080,
                x0: 0,
                x1: 1920,
                id: 0,
            }]
        );
    }

    #[test]
    fn a_window_top_becomes_a_ledge() {
        let l = ledges(&[client(100, 200, 400, 300)], 1920, 1080);
        assert!(l.iter().any(|g| (g.y, g.x0, g.x1) == (200, 100, 500)));
    }

    #[test]
    fn a_window_in_front_splits_the_ledge_behind_it() {
        // Front window sits above and overlaps the middle of the back one.
        let front = client(200, 100, 100, 400);
        let back = client(100, 200, 400, 300);
        let l = ledges(&[front, back], 1920, 1080);

        let spans: Vec<(i32, i32, i32)> = l.iter().map(|g| (g.y, g.x0, g.x1)).collect();
        assert!(
            spans.contains(&(200, 100, 200)),
            "left fragment missing: {spans:?}"
        );
        assert!(
            spans.contains(&(200, 300, 500)),
            "right fragment missing: {spans:?}"
        );
        assert!(
            !spans.contains(&(200, 100, 500)),
            "buried span still walkable"
        );
    }

    #[test]
    fn unmapped_windows_are_not_terrain() {
        let mut c = client(0, 0, 100, 100);
        c.mapped = false;
        assert_eq!(ledges(&[c], 1920, 1080).len(), 1);
    }

    #[test]
    fn a_stationary_object_never_looms() {
        assert_eq!(
            loom_drive((100.0, 100.0), (110.0, 100.0), (0.0, 0.0), 300.0),
            0.0
        );
    }

    #[test]
    fn a_receding_object_never_looms() {
        assert_eq!(
            loom_drive((100.0, 100.0), (150.0, 100.0), (900.0, 0.0), 300.0),
            0.0
        );
    }

    #[test]
    fn a_distant_object_never_looms() {
        assert_eq!(
            loom_drive((0.0, 0.0), (900.0, 0.0), (-2000.0, 0.0), 300.0),
            0.0
        );
    }

    #[test]
    fn closing_fast_and_near_looms_hardest() {
        let near = loom_drive((0.0, 0.0), (40.0, 0.0), (-1500.0, 0.0), 300.0);
        let far = loom_drive((0.0, 0.0), (250.0, 0.0), (-1500.0, 0.0), 300.0);
        let slow = loom_drive((0.0, 0.0), (40.0, 0.0), (-100.0, 0.0), 300.0);

        assert!(near > far, "near {near} should beat far {far}");
        assert!(near > slow, "fast {near} should beat slow {slow}");
        assert!(near <= 1.0);
    }

    #[test]
    fn tracker_reports_no_velocity_on_the_first_sample() {
        let mut t = CursorTracker::new();
        t.update(CursorPos { x: 10, y: 10 });
        assert_eq!(t.velocity(), (0.0, 0.0));
        assert!(!t.moved(1.0));
    }
}
