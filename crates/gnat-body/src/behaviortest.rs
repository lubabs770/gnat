//! The end-to-end suite: stimulate real neurons, watch the body react.
//!
//! A port of the original's `--behaviortest`. Seven scenarios drive the actual
//! circuit and check what the fly does about it; ten more hand-build signals to
//! exercise body mechanics that no amount of spiking would reach reliably.
//!
//! `--simtest` proves the circuit is alive. This proves it is *connected* — the
//! two suites are the original's ground truth and neither substitutes for the
//! other.

use crate::circadian;
use crate::fly::{FLY_SCALE, Fly, Ledge, State};
use crate::signals::{SignalBuilder, Signals};
use gnat_sim::{Circuit, Lif};

/// The original's window size, kept so the flight-distance constants behave.
const BOUNDS: (f32, f32) = (1512.0, 982.0);
const DT: f32 = 1.0 / 60.0;

pub struct Outcome {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    pub fn failures(&self) -> usize {
        self.outcomes.iter().filter(|o| !o.passed).count()
    }

    pub fn passed(&self) -> bool {
        self.failures() == 0
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for o in &self.outcomes {
            writeln!(
                f,
                "{}  {}: {}",
                if o.passed { "PASS" } else { "FAIL" },
                o.name,
                o.detail
            )?;
        }
        let failures = self.failures();
        if failures == 0 {
            write!(f, "ALL BEHAVIOR TESTS PASS ({})", self.outcomes.len())
        } else {
            write!(f, "{failures} FAILURES")
        }
    }
}

/// Drives the circuit, then watches the body for `hold` seconds.
struct Scenario {
    name: &'static str,
    hold: f32,
}

impl Scenario {
    fn run(
        self,
        circuit: Circuit,
        seed: u64,
        stim: impl FnOnce(&mut Lif),
        setup: impl FnOnce(&mut Fly),
        check: impl Fn(&Fly) -> bool,
        describe: impl Fn(&Fly) -> String,
    ) -> Outcome {
        let mut sim = Lif::new(circuit, seed);
        let mut builder = SignalBuilder::new();
        let mut fly = Fly::new((0.0, 0.0), seed);
        fly.state = State::Idle;
        fly.speed = 0.0;
        setup(&mut fly);

        // Settle the network and drain any startup giant-fibre latch, so the
        // scenario measures its own stimulus and not the switch-on transient.
        sim.step(400);
        let _ = sim.consume_gf();
        stim(&mut sim);

        let mut passed = false;
        for _ in 0..(self.hold / DT) as i32 {
            sim.step((DT * 1000.0).round() as i64);
            let s = builder.make(&mut sim, DT);
            fly.update(DT, BOUNDS, None, Some(s));
            if check(&fly) {
                passed = true;
                break;
            }
        }
        Outcome {
            name: self.name,
            passed,
            detail: describe(&fly),
        }
    }
}

fn body_check(name: &'static str, run: impl FnOnce() -> (bool, String)) -> Outcome {
    let (passed, detail) = run();
    Outcome {
        name,
        passed,
        detail,
    }
}

fn walk_signals() -> Signals {
    Signals {
        walk_drive: 0.6,
        ..Signals::default()
    }
}

pub fn run(circuit: &Circuit, seed: u64) -> Report {
    let clone = || circuit.clone();
    let groups = Lif::new(clone(), seed).groups().clone();
    let mut outcomes = Vec::new();

    // ---- circuit-driven scenarios -----------------------------------------
    outcomes.push(
        Scenario {
            name: "GF stim -> escape flight",
            hold: 0.5,
        }
        .run(
            clone(),
            seed,
            |s| s.stimulate(&groups.gf, 0.5, 40),
            |_| {},
            |f| f.state == State::Flying,
            |f| format!("state={:?}", f.state),
        ),
    );

    outcomes.push(
        Scenario {
            name: "DNg11 stim -> grooming",
            hold: 1.5,
        }
        .run(
            clone(),
            seed,
            |s| s.stimulate(&groups.groom, 0.25, 600),
            |_| {},
            |f| f.state == State::Grooming,
            |f| format!("state={:?}", f.state),
        ),
    );

    outcomes.push(
        Scenario {
            name: "DNp09 stim -> walks, speed rises (capped)",
            hold: 1.5,
        }
        .run(
            clone(),
            seed,
            |s| s.stimulate(&groups.fwd, 0.25, 1200),
            |_| {},
            |f| f.state == State::Walking && f.speed > 40.0 && f.speed < 100.0,
            |f| format!("state={:?} speed={}", f.state, f.speed as i32),
        ),
    );

    outcomes.push(
        Scenario {
            name: "MDN stim (from idle) -> backward walk",
            hold: 1.2,
        }
        .run(
            clone(),
            seed,
            |s| s.stimulate(&groups.mdn, 0.3, 600),
            |_| {},
            |f| f.backward_timer > 0.0,
            |f| format!("backward_timer={:.2}", f.backward_timer),
        ),
    );

    outcomes.push(
        Scenario {
            name: "DNa-left stim -> left (CCW) turn while walking",
            hold: 1.4,
        }
        .run(
            clone(),
            seed,
            |s| s.stimulate(&groups.dna_l, 0.3, 900),
            |f| {
                f.state = State::Walking;
                f.speed = 30.0;
                f.heading = 0.0;
            },
            |f| f.heading > 0.25,
            |f| format!("heading change {:+.2} rad", f.heading),
        ),
    );

    outcomes.push(
        Scenario {
            name: "moderate loom -> fear response (dart or escape)",
            hold: 1.0,
        }
        .run(
            clone(),
            seed,
            |s| {
                s.inputs.loom_l = 0.45;
                s.inputs.loom_r = 0.45;
            },
            |_| {},
            |f| (f.state == State::Walking && f.speed > 100.0) || f.state == State::Flying,
            |f| format!("state={:?} speed={}", f.state, f.speed as i32),
        ),
    );

    outcomes.push(
        Scenario {
            name: "tap near fly -> startle escape via sensory pathway",
            hold: 0.8,
        }
        .run(
            clone(),
            seed,
            |s| s.stimulate(&groups.sens, 0.45, 150),
            |_| {},
            |f| f.state == State::Flying,
            |f| format!("state={:?}", f.state),
        ),
    );

    // ---- body-level checks, on hand-built signals --------------------------
    outcomes.push(body_check("ledge attach + follow window edge", || {
        let mut fly = Fly::new((0.0, -55.0), seed);
        fly.state = State::Walking;
        fly.speed = 30.0;
        fly.heading = 0.0;
        fly.terrain = vec![Ledge {
            y: -40.0,
            x0: -300.0,
            x1: 300.0,
            id: 1,
        }];
        for _ in 0..240 {
            fly.update(DT, BOUNDS, None, Some(walk_signals()));
            if fly.ledge.is_some() && (fly.pos.1 + 40.0).abs() < 8.0 {
                return (true, format!("attached, y={}", fly.pos.1 as i32));
            }
        }
        (
            false,
            format!(
                "state={:?} y={} ledge={}",
                fly.state,
                fly.pos.1 as i32,
                fly.ledge.is_some()
            ),
        )
    }));

    outcomes.push(body_check("window closes underfoot -> takeoff", || {
        let mut fly = Fly::new((0.0, -40.0), seed);
        fly.state = State::Walking;
        fly.speed = 25.0;
        fly.heading = 0.0;
        let ledge = Ledge {
            y: -40.0,
            x0: -300.0,
            x1: 300.0,
            id: 1,
        };
        fly.ledge = Some(ledge);
        // The window is gone; only the fly's belief in it remains.
        fly.terrain = Vec::new();
        for _ in 0..60 {
            fly.update(DT, BOUNDS, None, Some(walk_signals()));
            if fly.state == State::Flying {
                return (true, "took off".into());
            }
        }
        (false, format!("state={:?}", fly.state))
    }));

    outcomes.push(body_check(
        "sleep signal -> sleeping; wake -> grooming",
        || {
            let mut fly = Fly::new((0.0, 0.0), seed);
            fly.state = State::Idle;
            let mut s = Signals {
                sleep: true,
                ..Signals::default()
            };
            for _ in 0..60 {
                fly.update(DT, BOUNDS, None, Some(s));
            }
            if fly.state != State::Sleeping {
                return (false, format!("no sleep: {:?}", fly.state));
            }
            s.sleep = false;
            fly.update(DT, BOUNDS, None, Some(s));
            (
                fly.state == State::Grooming,
                format!("woke to {:?}", fly.state),
            )
        },
    ));

    outcomes.push(body_check("thermal tempo scales walking speed", || {
        let mut fly = Fly::new((0.0, 0.0), seed);
        fly.state = State::Walking;
        fly.speed = 20.0;
        fly.heading = 0.0;
        let cool = Signals {
            tempo: 1.0,
            ..walk_signals()
        };
        for _ in 0..120 {
            fly.update(DT, BOUNDS, None, Some(cool));
        }
        let cool_speed = fly.speed;
        let hot = Signals {
            tempo: 1.5,
            ..walk_signals()
        };
        for _ in 0..120 {
            fly.update(DT, BOUNDS, None, Some(hot));
        }
        let hot_speed = fly.speed;
        (
            fly.state == State::Walking && hot_speed > cool_speed + 10.0,
            format!(
                "cool {} -> hot {} pt/s",
                cool_speed as i32, hot_speed as i32
            ),
        )
    }));

    outcomes.push(body_check(
        "flight: altitude drives scale; escape flies higher than casual",
        || {
            let flight = |escape: bool, effort: Option<f32>| -> (f32, f32) {
                let mut fly = Fly::new((0.0, 0.0), seed);
                fly.state = State::Idle;
                fly.start_flight(BOUNDS, None, escape, effort);
                let (mut max_alt, mut max_scale) = (0.0f32, 0.0f32);
                let mut frames = 0;
                while fly.state == State::Flying && frames < 400 {
                    frames += 1;
                    fly.update(DT, BOUNDS, None, None);
                    max_alt = max_alt.max(fly.alt);
                    max_scale = max_scale.max(fly.scale);
                }
                (max_alt, max_scale)
            };
            let esc = flight(true, None);
            let casual = flight(false, Some(0.45));
            let ok = esc.0 > casual.0 + 0.15
                && esc.1 > FLY_SCALE * 1.5
                // Scale must be a pure function of altitude, not an independent
                // animation that can drift out of step with it.
                && (esc.1 - FLY_SCALE * (1.0 + 0.8 * esc.0)).abs() < 0.15;
            (
                ok,
                format!(
                    "escape alt {:.2} scale {:.2} | casual alt {:.2} scale {:.2}",
                    esc.0, esc.1, casual.0, casual.1
                ),
            )
        },
    ));

    outcomes.push(body_check("flight: wings actually beat", || {
        let mut fly = Fly::new((0.0, 0.0), seed);
        fly.state = State::Idle;
        fly.start_flight(BOUNDS, None, false, Some(0.8));
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..30 {
            if fly.state != State::Flying {
                break;
            }
            fly.update(DT, BOUNDS, None, None);
            lo = lo.min(fly.wings[0].z);
            hi = hi.max(fly.wings[0].z);
        }
        (
            hi - lo > 0.25,
            format!("wing sweep {:.2} rad over 0.5 s", hi - lo),
        )
    }));

    outcomes.push(body_check(
        "escape-DN activity mid-flight raises wing-beat effort",
        || {
            let mut fly = Fly::new((0.0, 0.0), seed);
            fly.state = State::Idle;
            fly.start_flight(BOUNDS, None, false, Some(0.5));
            let calm = Signals::default();
            for _ in 0..12 {
                fly.update(DT, BOUNDS, None, Some(calm));
            }
            let calm_effort = fly.effort_current;
            let hot = Signals {
                wing_drive: 1.0,
                arousal: 0.6,
                ..Signals::default()
            };
            for _ in 0..12 {
                if fly.state != State::Flying {
                    break;
                }
                fly.update(DT, BOUNDS, None, Some(hot));
            }
            (
                fly.state == State::Flying && fly.effort_current > calm_effort + 0.2,
                format!("effort {:.2} -> {:.2}", calm_effort, fly.effort_current),
            )
        },
    ));

    outcomes.push(body_check(
        "threat while grounded raises the wings (no takeoff)",
        || {
            let mut fly = Fly::new((0.0, 0.0), seed);
            fly.state = State::Walking;
            fly.speed = 20.0;
            // Isolate the posture from darting.
            fly.dart_cooldown = 99.0;
            let threat = Signals {
                wing_drive: 0.9,
                walk_drive: 0.4,
                ..Signals::default()
            };
            for _ in 0..40 {
                fly.update(DT, BOUNDS, None, Some(threat));
            }
            (
                fly.state != State::Flying && fly.wing_raise > 0.6 && fly.wings[0].x < -0.2,
                format!(
                    "raise {:.2}, wing tilt {:.2} rad",
                    fly.wing_raise, fly.wings[0].x
                ),
            )
        },
    ));

    outcomes.push(body_check(
        "landing is smooth: no scale/height snap at touchdown",
        || {
            let mut fly = Fly::new((0.0, 0.0), seed);
            fly.state = State::Idle;
            fly.start_flight(BOUNDS, None, true, None);
            let (mut prev_scale, mut prev_z) = (fly.scale, fly.z);
            let (mut max_ds, mut max_dz) = (0.0f32, 0.0f32);
            let mut post = 20;
            let mut frames = 0;
            let mut landed = false;
            while post > 0 && frames < 600 {
                frames += 1;
                fly.update(DT, BOUNDS, None, None);
                max_ds = max_ds.max((fly.scale - prev_scale).abs());
                max_dz = max_dz.max((fly.z - prev_z).abs());
                prev_scale = fly.scale;
                prev_z = fly.z;
                if fly.state != State::Flying {
                    landed = true;
                    post -= 1;
                }
            }
            (
                landed && max_ds < 0.2 && max_dz < 25.0,
                format!(
                    "landed={}, max per-frame d-scale {:.2}, d-z {:.1}",
                    if landed { "yes" } else { "NO" },
                    max_ds,
                    max_dz
                ),
            )
        },
    ));

    outcomes.push(body_check(
        "circadian curve: siesta + night dips, dawn/dusk peaks",
        || {
            let night = circadian::activity(3.0);
            let dawn = circadian::activity(9.0);
            let siesta = circadian::activity(14.0);
            let dusk = circadian::activity(18.0);
            let ok = night < 0.4 && dawn > 0.9 && (0.3..0.7).contains(&siesta) && dusk > 0.9;
            (
                ok,
                format!("3h {night:.2}, 9h {dawn:.2}, 14h {siesta:.2}, 18h {dusk:.2}"),
            )
        },
    ));

    Report { outcomes }
}
