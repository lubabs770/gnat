//! The fly's body and behaviour.
//!
//! A port of `FlyModel.swift`'s `Fly`, with the SceneKit node graph replaced by
//! plain state — position, scale, height, leg angles, wing angles. The renderer
//! reads those; nothing here knows what a scene graph is, which is what lets
//! the whole behaviour suite run headless.
//!
//! Coordinates are the original's scene frame: origin at the centre of the
//! output, **+y up**. That is not the screen frame `gnat-senses` reports in, and
//! converting between them is the sense-wiring step, not this crate's job.

use crate::signals::Signals;
use gnat_sim::Rng;

/// Rendered size of the fly at ground level.
pub const FLY_SCALE: f32 = 1.15;
/// How far from the edge of the output a flight target may land.
pub const EDGE_MARGIN: f32 = 50.0;
/// Legacy (non-connectome) fear radii, used only when there are no signals.
pub const SCARE_RADIUS: f32 = 110.0;
pub const NERVOUS_RADIUS: f32 = 240.0;

fn angle_diff(from: f32, to: f32) -> f32 {
    let mut d = (to - from) % (2.0 * std::f32::consts::PI);
    if d > std::f32::consts::PI {
        d -= 2.0 * std::f32::consts::PI;
    }
    if d < -std::f32::consts::PI {
        d += 2.0 * std::f32::consts::PI;
    }
    d
}

fn smoothstep(t: f32) -> f32 {
    let x = t.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn hypot(dx: f32, dy: f32) -> f32 {
    (dx * dx + dy * dy).sqrt()
}

/// A walkable window top edge, in scene coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ledge {
    pub y: f32,
    pub x0: f32,
    pub x1: f32,
    /// Stable identity of the window this edge belongs to. The fly re-finds
    /// its footing by id every frame, so a window that closes is noticed.
    pub id: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Walking,
    Idle,
    Grooming,
    Flying,
    Sleeping,
}

/// One leg's animation state. Six of them, tripod-phased.
#[derive(Clone, Copy, Debug)]
pub struct Leg {
    pub base_yaw: f32,
    /// +1 right, -1 left. Mirrors the swing direction.
    pub swing_sign: f32,
    /// Offset into the gait cycle, 0..1.
    pub phase: f32,
    pub is_front: bool,
    pub angle: f32,
    pub lift: f32,
}

/// Attachment yaw offset, gait phase, and whether it is a foreleg — the rest
/// of the original's leg spec is geometry the renderer owns.
const LEG_SPECS: [(f32, f32, f32, bool); 6] = [
    (1.0, 0.95, 0.0, true),
    (-1.0, 0.95, 0.5, true),
    (1.0, -0.10, 0.5, false),
    (-1.0, -0.10, 0.0, false),
    (1.0, -0.95, 0.0, false),
    (-1.0, -0.95, 0.5, false),
];

/// Euler angles of one wing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wing {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub struct Fly {
    pub pos: (f32, f32),
    pub heading: f32,
    pub speed: f32,
    pub state: State,
    pub state_timer: f32,
    pub gait_phase: f32,
    pub time: f32,
    pub scare_cooldown: f32,
    pub dart_cooldown: f32,
    pub backward_timer: f32,
    pub dart_timer: f32,
    pub state_age: f32,

    /// Walkable window edges, refreshed by the coordinator.
    pub terrain: Vec<Ledge>,
    /// The edge currently underfoot.
    pub ledge: Option<Ledge>,

    pub flight_from: (f32, f32),
    pub flight_to: (f32, f32),
    pub flight_t: f32,
    pub flight_dur: f32,
    /// Set at takeoff: 1.0 for escape, lower for a casual hop.
    pub flight_effort: f32,
    /// Live effort: the base, plus ongoing escape-DN and arousal drive.
    pub effort_current: f32,
    /// 0 on the ground, 1 at maximum altitude.
    pub alt: f32,
    pub pitch: f32,
    pub flap_phase: f32,
    /// Grounded threat posture, 0..1.
    pub wing_raise: f32,

    /// Render scale. Higher altitude reads as closer to the viewer.
    pub scale: f32,
    /// Height above the surface, in scene units.
    pub z: f32,
    /// Abdomen breathing multiplier, slower and deeper while asleep.
    pub breath: f32,

    pub legs: [Leg; 6],
    /// Left wing then right wing.
    pub wings: [Wing; 2],

    brain_live: bool,
    live_arousal: f32,
    live_wing: f32,
    rng: Rng,
}

impl Fly {
    pub fn new(at: (f32, f32), seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let legs = std::array::from_fn(|i| {
            let (side, yaw_off, phase, is_front) = LEG_SPECS[i];
            Leg {
                base_yaw: if side > 0.0 {
                    yaw_off
                } else {
                    std::f32::consts::PI - yaw_off
                },
                swing_sign: side,
                phase,
                is_front,
                angle: 0.0,
                lift: 0.0,
            }
        });

        Self {
            pos: at,
            heading: rng.range(0.0, 2.0 * std::f32::consts::PI),
            speed: 30.0,
            state: State::Walking,
            state_timer: rng.range(1.5, 4.0),
            gait_phase: rng.range(0.0, 1.0),
            time: rng.range(0.0, 100.0),
            scare_cooldown: 0.0,
            dart_cooldown: 0.0,
            backward_timer: 0.0,
            dart_timer: 0.0,
            state_age: 0.0,
            terrain: Vec::new(),
            ledge: None,
            flight_from: (0.0, 0.0),
            flight_to: (0.0, 0.0),
            flight_t: 0.0,
            flight_dur: 1.0,
            flight_effort: 0.6,
            effort_current: 0.6,
            alt: 0.0,
            pitch: 0.0,
            flap_phase: 0.0,
            wing_raise: 0.0,
            scale: FLY_SCALE,
            z: 0.0,
            breath: 1.0,
            legs,
            // Folded flat over the abdomen, the resting pose. `land()` sets
            // this too, but a fly that starts on the ground never lands, and
            // an unposed wing is an invisible one.
            wings: [
                Wing {
                    x: 0.0,
                    y: 0.0,
                    z: -0.13,
                },
                Wing {
                    x: 0.0,
                    y: 0.0,
                    z: 0.13,
                },
            ],
            brain_live: false,
            live_arousal: 0.0,
            live_wing: 0.0,
            rng,
        }
    }

    /// Gait cycle position, fed back into the sim's ascending neurons.
    pub fn gait_phase(&self) -> f32 {
        self.gait_phase
    }

    /// How hard the fly is walking, 0..1, likewise fed back to the brain.
    pub fn walking_intensity(&self) -> f32 {
        if self.state == State::Walking {
            (self.effective_speed().abs() / 60.0).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn effective_speed(&self) -> f32 {
        if self.backward_timer > 0.0 {
            -22.0
        } else {
            self.speed
        }
    }

    pub fn start_flight(
        &mut self,
        bounds: (f32, f32),
        away_from: Option<(f32, f32)>,
        escape: bool,
        effort: Option<f32>,
    ) {
        self.state = State::Flying;
        self.ledge = None;
        self.flight_effort = effort
            .unwrap_or(if escape {
                1.0
            } else {
                self.rng.range(0.4, 0.75)
            })
            .clamp(0.25, 1.0);
        self.effort_current = self.flight_effort;
        self.flap_phase = 0.0;
        self.wing_raise = 0.0;
        self.flight_from = self.pos;

        let hw = bounds.0 / 2.0 - EDGE_MARGIN;
        let hh = bounds.1 / 2.0 - EDGE_MARGIN;
        let mut target = (0.0, 0.0);
        let mut chosen = false;

        // A casual flight often ends on a window edge rather than open ground.
        if !escape && away_from.is_none() && !self.terrain.is_empty() && self.rng.f32() < 0.45 {
            let l = self.terrain[self.rng.range_i(0, self.terrain.len() as i64 - 1) as usize];
            if l.x1 - l.x0 > 90.0 {
                target = (self.rng.range(l.x0 + 25.0, l.x1 - 25.0), l.y);
                chosen = hypot(target.0 - self.pos.0, target.1 - self.pos.1) > 180.0;
            }
        }
        if !chosen {
            for _ in 0..16 {
                target = (self.rng.range(-hw, hw), self.rng.range(-hh, hh));
                let far = hypot(target.0 - self.pos.0, target.1 - self.pos.1)
                    > if escape { 350.0 } else { 260.0 };
                if !far {
                    continue;
                }
                if let Some(a) = away_from {
                    // Escaping means landing on the far side of the fly from
                    // the threat, not merely somewhere distant.
                    let to_t = (target.0 - self.pos.0, target.1 - self.pos.1);
                    let to_a = (a.0 - self.pos.0, a.1 - self.pos.1);
                    if to_t.0 * to_a.0 + to_t.1 * to_a.1 > 0.0 {
                        continue;
                    }
                }
                break;
            }
        }

        self.flight_to = target;
        let dist = hypot(target.0 - self.pos.0, target.1 - self.pos.1);
        self.flight_dur = if escape {
            (dist / 650.0).clamp(0.45, 1.2)
        } else {
            (dist / 420.0).clamp(0.7, 2.0)
        };
        self.flight_t = 0.0;
        self.scare_cooldown = if escape { 2.0 } else { 2.5 };
    }

    fn land(&mut self) {
        self.state = State::Idle;
        self.state_timer = self.rng.range(0.3, 0.8);
        self.speed = 0.0;
        self.alt = 0.0;
        self.pitch = 0.0;
        self.scale = FLY_SCALE;
        self.z = 0.0;
        // Refold the wings flat over the abdomen.
        for (i, wing) in self.wings.iter_mut().enumerate() {
            let side = if i == 0 { -1.0 } else { 1.0 };
            *wing = Wing {
                x: 0.0,
                y: 0.0,
                z: side * 0.13,
            };
        }
    }

    fn pick_next_state(&mut self) {
        match self.state {
            State::Walking => {
                let r = self.rng.f32();
                if r < 0.30 {
                    self.state = State::Idle;
                    self.state_timer = self.rng.range(0.8, 3.0);
                    self.speed = 0.0;
                } else if r < 0.55 {
                    self.state_timer = self.rng.range(0.3, 0.8);
                    self.speed = self.rng.range(95.0, 150.0);
                    self.heading += self.rng.range(-1.2, 1.2);
                } else {
                    self.state_timer = self.rng.range(1.5, 5.0);
                    self.speed = self.rng.range(18.0, 45.0);
                }
            }
            State::Idle => {
                let r = self.rng.f32();
                if r < 0.35 {
                    self.state = State::Grooming;
                    self.state_timer = self.rng.range(1.0, 2.5);
                } else {
                    self.state = State::Walking;
                    self.state_timer = self.rng.range(1.5, 5.0);
                    self.speed = self.rng.range(18.0, 45.0);
                    self.heading += self.rng.range(-1.5, 1.5);
                }
            }
            State::Grooming => {
                self.state = State::Idle;
                self.state_timer = self.rng.range(0.3, 1.0);
            }
            State::Flying | State::Sleeping => {}
        }
    }

    fn set_state(&mut self, s: State) {
        if s == self.state {
            return;
        }
        self.state = s;
        self.state_age = 0.0;
    }

    pub fn update(
        &mut self,
        dt: f32,
        bounds: (f32, f32),
        mouse: Option<(f32, f32)>,
        signals: Option<Signals>,
    ) {
        self.time += dt;
        self.scare_cooldown = (self.scare_cooldown - dt).max(0.0);
        self.dart_cooldown = (self.dart_cooldown - dt).max(0.0);
        self.backward_timer = (self.backward_timer - dt).max(0.0);
        self.state_age += dt;
        self.dart_timer = (self.dart_timer - dt).max(0.0);

        // Live brain drives reach the wings even mid-flight.
        self.brain_live = signals.is_some();
        self.live_arousal = signals.map_or(0.0, |s| s.arousal);
        self.live_wing = signals.map_or(0.0, |s| s.wing_drive);

        if self.state == State::Flying {
            self.update_flight(dt);
        } else if let Some(s) = signals {
            self.brain_behavior(s, dt, bounds, mouse);
            if self.state == State::Walking {
                self.update_walk(dt, bounds);
            }
        } else {
            self.legacy_behavior(dt, bounds, mouse);
        }

        self.update_legs(dt);
        self.update_wings(dt);
        // Slower, deeper breathing while asleep.
        self.breath = if self.state == State::Sleeping {
            1.0 + 0.05 * (self.time * 1.1).sin()
        } else {
            1.0 + 0.03 * (self.time * 3.0).sin()
        };
    }

    /// Mouse-distance fear, for extra flies that have no brain of their own.
    fn legacy_behavior(&mut self, dt: f32, bounds: (f32, f32), mouse: Option<(f32, f32)>) {
        if self.scare_cooldown == 0.0
            && let Some(m) = mouse
        {
            let d = hypot(m.0 - self.pos.0, m.1 - self.pos.1);
            if d < SCARE_RADIUS {
                self.start_flight(bounds, Some(m), false, None);
            } else if d < NERVOUS_RADIUS && self.state != State::Walking {
                self.set_state(State::Walking);
                self.heading =
                    (self.pos.1 - m.1).atan2(self.pos.0 - m.0) + self.rng.range(-0.4, 0.4);
                self.speed = self.rng.range(110.0, 150.0);
                self.state_timer = self.rng.range(0.4, 0.9);
                self.scare_cooldown = 1.0;
            }
        }
        if self.state != State::Flying {
            self.state_timer -= dt;
            if self.state_timer <= 0.0 {
                if self.state == State::Walking && self.rng.f32() < 0.10 {
                    self.start_flight(bounds, None, false, None);
                } else {
                    self.pick_next_state();
                }
            }
            if self.state == State::Walking {
                self.update_walk(dt, bounds);
            }
        }
    }

    /// Every decision here reads a real neuron population's rate.
    fn brain_behavior(
        &mut self,
        s: Signals,
        dt: f32,
        bounds: (f32, f32),
        mouse: Option<(f32, f32)>,
    ) {
        // Giant fibre spike: escape takeoff, even out of sleep.
        if s.escape && self.scare_cooldown == 0.0 {
            self.start_flight(bounds, mouse, true, None);
            return;
        }
        // Circadian sleep: enter, hold, and wake into grooming the way a real
        // fly does.
        if s.sleep {
            if self.state != State::Sleeping {
                self.set_state(State::Sleeping);
                self.speed = 0.0;
                self.dart_timer = 0.0;
                self.backward_timer = 0.0;
            }
            return;
        } else if self.state == State::Sleeping {
            self.set_state(State::Grooming);
            return;
        }
        // Looming detectors hot but the giant fibre quiet: a nervous dart
        // rather than a full takeoff.
        if s.nervous > 0.40 && self.dart_cooldown == 0.0 {
            self.ledge = None;
            self.set_state(State::Walking);
            self.heading = match mouse {
                Some(m) => (self.pos.1 - m.1).atan2(self.pos.0 - m.0) + self.rng.range(-0.4, 0.4),
                None => self.heading + self.rng.range(-1.5, 1.5),
            };
            self.speed = self.rng.range(110.0, 155.0);
            self.dart_timer = self.rng.range(0.4, 0.9);
            self.dart_cooldown = 1.2;
        }
        // DNg11 grooming command, with hysteresis so it does not chatter.
        if self.state != State::Walking || self.dart_timer == 0.0 {
            if self.state != State::Grooming
                && s.groom_drive > 0.5
                && s.nervous < 0.3
                && self.state_age > 0.4
            {
                self.set_state(State::Grooming);
            } else if self.state == State::Grooming && s.groom_drive < 0.3 && self.state_age > 0.6 {
                self.set_state(State::Idle);
            }
        }
        // DNp09 forward-walking command, likewise hysteretic.
        if self.state == State::Idle && s.walk_drive > 0.22 && self.state_age > 0.4 {
            self.set_state(State::Walking);
            self.heading += self.rng.range(-0.8, 0.8);
        } else if self.state == State::Walking
            && self.dart_timer == 0.0
            && s.walk_drive < 0.08
            && self.state_age > 0.5
        {
            self.set_state(State::Idle);
            self.speed = 0.0;
        }
        // An MDN burst reverses the fly from any grounded state.
        if s.backward && self.backward_timer == 0.0 && self.dart_timer == 0.0 {
            if self.state != State::Walking {
                self.set_state(State::Walking);
                self.speed = 0.0;
            }
            self.backward_timer = 0.5;
        }
        // Walking speed follows the forward command; tempo is temperature.
        if self.state == State::Walking {
            if self.dart_timer == 0.0 && self.backward_timer == 0.0 {
                let target = (14.0 + s.walk_drive * 55.0) * s.tempo;
                self.speed += (target - self.speed) * (3.0 * dt).min(1.0);
            }
            if self.ledge.is_none() {
                // DNa01/DNa02 steering.
                self.heading += s.turn_bias * dt;
            }
        }
        // Spontaneous takeoff, gated on whole-population arousal. Flight
        // effort scales with how aroused the network is.
        let flight_chance = if s.arousal > 0.5 { 0.6 } else { 0.005 };
        if self.state == State::Walking && self.rng.f32() < flight_chance * dt {
            let effort = 0.35 + s.arousal * 0.6;
            self.start_flight(bounds, None, false, Some(effort));
        }
    }

    fn update_walk(&mut self, dt: f32, bounds: (f32, f32)) {
        // Re-find the attached ledge: windows move and close underfoot.
        if let Some(l) = self.ledge {
            match self
                .terrain
                .iter()
                .find(|c| c.id == l.id)
                .filter(|c| (c.y - l.y).abs() < 40.0)
            {
                Some(cur) => self.ledge = Some(*cur),
                None => {
                    self.ledge = None;
                    // The ground vanished from under it.
                    self.start_flight(bounds, None, false, None);
                    return;
                }
            }
        }

        if let Some(l) = self.ledge {
            // Walk along the window edge, snapping heading to the axis.
            self.heading += self.rng.range(-1.0, 1.0) * 0.2 * dt;
            let along = if self.heading.cos() >= 0.0 {
                0.0
            } else {
                std::f32::consts::PI
            };
            self.heading += angle_diff(self.heading, along) * (6.0 * dt).min(1.0);
            self.pos.0 += self.heading.cos() * self.effective_speed() * dt;
            self.pos.1 += (l.y - self.pos.1) * (10.0 * dt).min(1.0);
            if self.pos.0 <= l.x0 + 6.0 && self.heading.cos() < 0.0 {
                self.heading = 0.0;
            }
            if self.pos.0 >= l.x1 - 6.0 && self.heading.cos() > 0.0 {
                self.heading = std::f32::consts::PI;
            }
            self.pos.0 = self.pos.0.clamp(l.x0, l.x1);
            if self.rng.f32() < 0.05 * dt {
                // Wander off the edge.
                self.ledge = None;
            }
        } else {
            self.heading += self.rng.range(-1.0, 1.0) * 1.6 * dt;
            let hw = bounds.0 / 2.0 - EDGE_MARGIN;
            let hh = bounds.1 / 2.0 - EDGE_MARGIN;
            if self.pos.0.abs() > hw || self.pos.1.abs() > hh {
                let to_centre = (-self.pos.1).atan2(-self.pos.0);
                self.heading += angle_diff(self.heading, to_centre) * (4.0 * dt).min(1.0);
            }
            let v = self.effective_speed();
            self.pos.0 += self.heading.cos() * v * dt;
            self.pos.1 += self.heading.sin() * v * dt;
            self.pos.0 = self
                .pos
                .0
                .clamp(-bounds.0 / 2.0 + 20.0, bounds.0 / 2.0 - 20.0);
            self.pos.1 = self
                .pos
                .1
                .clamp(-bounds.1 / 2.0 + 20.0, bounds.1 / 2.0 - 20.0);

            // Walked onto a window edge? Latch on.
            let hit = self
                .terrain
                .iter()
                .find(|l| {
                    self.pos.0 > l.x0 - 8.0
                        && self.pos.0 < l.x1 + 8.0
                        && (self.pos.1 - l.y).abs() < 20.0
                })
                .copied();
            if let Some(l) = hit
                && self.rng.f32() < 0.9 * dt
            {
                self.ledge = Some(l);
                self.heading = if self.heading.cos() >= 0.0 {
                    0.0
                } else {
                    std::f32::consts::PI
                };
            }
        }
        self.z = 0.35 * (self.gait_phase * std::f32::consts::PI * 2.0).sin().abs();
    }

    fn apply_altitude(&mut self) {
        self.scale = FLY_SCALE * (1.0 + 0.8 * self.alt);
        self.z = 90.0 * self.alt;
    }

    fn update_flight(&mut self, dt: f32) {
        self.flight_t = (self.flight_t + dt / self.flight_dur).min(1.0);
        if self.flight_t >= 1.0 {
            // Touchdown flare: the timer is up, but the fly only lands once it
            // has actually descended. Hover over the target and settle.
            self.pos.0 = self.flight_to.0 + (self.time * 26.0).sin() * 1.2;
            self.pos.1 = self.flight_to.1 + (self.time * 22.0).cos() * 1.0;
            self.pitch = (self.alt * 0.4).clamp(0.0, 0.35);
            self.alt += (0.0 - self.alt) * (9.0 * dt).min(1.0);
            self.apply_altitude();
            if self.alt < 0.035 {
                self.pos = self.flight_to;
                self.land();
            }
            return;
        }

        let e = smoothstep(self.flight_t);
        let dx = self.flight_to.0 - self.flight_from.0;
        let dy = self.flight_to.1 - self.flight_from.1;
        let len = hypot(dx, dy).max(1.0);
        let (px, py) = (-dy / len, dx / len);
        let wob = (self.time * 32.0).sin() * 4.0 * (self.flight_t * std::f32::consts::PI).sin();
        self.pos.0 = self.flight_from.0 + dx * e + px * wob;
        self.pos.1 = self.flight_from.1 + dy * e + py * wob;
        self.heading = dy.atan2(dx) + (self.time * 18.0).sin() * 0.12;

        // Effort stays live: ongoing escape-DN and arousal activity make the
        // fly beat harder and climb higher part-way through a flight.
        self.effort_current = if self.brain_live {
            self.flight_effort
                .max(self.flight_effort * 0.55 + self.live_arousal * 0.25 + self.live_wing * 0.6)
                .clamp(0.25, 1.3)
        } else {
            self.flight_effort
        };
        let rise_env = (self.flight_t / 0.25).min(1.0);
        let fall_env = ((1.0 - self.flight_t) / 0.3).min(1.0);
        let target =
            self.effort_current * rise_env.min(fall_env) * (0.85 + 0.15 * (self.time * 7.0).sin());
        self.pitch = ((target - self.alt) * 2.5).clamp(-0.45, 0.45);
        self.alt += (target - self.alt) * (6.0 * dt).min(1.0);
        self.apply_altitude();
    }

    fn update_legs(&mut self, dt: f32) {
        let v = self.effective_speed().abs();
        let walking = self.state == State::Walking && v > 1.0;

        if walking {
            let amp = (0.20 + v * 0.0022).clamp(0.20, 0.50);
            let stride = (2.0 * amp * 13.0).max(5.0);
            let freq = (v / stride).clamp(3.0, 11.0);
            self.gait_phase = (self.gait_phase + freq * dt) % 1.0;
            // Tripod gait: three legs in stance while three swing.
            const STANCE_FRAC: f32 = 0.6;
            let backward = self.backward_timer > 0.0;
            let phase = self.gait_phase;
            for leg in &mut self.legs {
                let p = (phase + leg.phase) % 1.0;
                if p < STANCE_FRAC {
                    leg.angle = amp * (1.0 - 2.0 * (p / STANCE_FRAC));
                    leg.lift = 0.0;
                } else {
                    let s = (p - STANCE_FRAC) / (1.0 - STANCE_FRAC);
                    leg.angle = -amp + 2.0 * amp * smoothstep(s);
                    leg.lift = (s * std::f32::consts::PI).sin() * 0.55;
                }
                if backward {
                    leg.angle = -leg.angle;
                }
            }
        } else if self.state == State::Grooming {
            let time = self.time;
            for leg in &mut self.legs {
                if leg.is_front {
                    leg.angle = 0.45 + 0.25 * (time * 20.0 + leg.swing_sign * 1.3).sin();
                    leg.lift = 0.55 + 0.15 * (time * 22.0).sin();
                } else {
                    leg.angle += (0.0 - leg.angle) * (8.0 * dt).min(1.0);
                    leg.lift += (0.0 - leg.lift) * (8.0 * dt).min(1.0);
                }
            }
        } else if self.state == State::Flying {
            for leg in &mut self.legs {
                leg.angle += (-0.35 - leg.angle) * (6.0 * dt).min(1.0);
                leg.lift += (0.5 - leg.lift) * (6.0 * dt).min(1.0);
            }
        } else {
            for leg in &mut self.legs {
                leg.angle += (0.0 - leg.angle) * (10.0 * dt).min(1.0);
                leg.lift += (0.0 - leg.lift) * (10.0 * dt).min(1.0);
            }
        }
    }

    fn update_wings(&mut self, dt: f32) {
        if self.state != State::Flying {
            // Grounded threat posture: escape-DN or loom activity raises the
            // wings without taking off.
            let raise_target = if self.state != State::Sleeping
                && (self.live_wing > 0.7 || (self.brain_live && self.dart_timer > 0.0))
            {
                1.0
            } else {
                0.0
            };
            self.wing_raise += (raise_target - self.wing_raise) * (8.0 * dt).min(1.0);
            if self.wing_raise > 0.01 {
                let raise = self.wing_raise;
                for (i, wing) in self.wings.iter_mut().enumerate() {
                    let side = if i == 0 { -1.0 } else { 1.0 };
                    *wing = Wing {
                        x: -0.5 * raise,
                        y: 0.0,
                        z: side * (0.13 + 0.3 * raise),
                    };
                }
            }
            return;
        }

        // Visible wing beat: the stroke arc sweeps faster at higher effort.
        self.flap_phase = (self.flap_phase + dt * (14.0 + 10.0 * self.effort_current)) % 1.0;
        let stroke = (self.flap_phase * 2.0 * std::f32::consts::PI).sin();
        for (i, wing) in self.wings.iter_mut().enumerate() {
            let side = if i == 0 { -1.0 } else { 1.0 };
            *wing = Wing {
                x: stroke * 0.35,
                y: 0.0,
                z: side * (0.45 + 0.35 * (0.5 + 0.5 * stroke)),
            };
        }
    }
}
