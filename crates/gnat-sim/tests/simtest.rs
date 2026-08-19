//! The circuit invariants, run against the real FlyWire v783 data.
//!
//! Bounds come from the original's own documentation ("GF silent over 4 s of
//! rest, GF fires <= ~10 ms after abrupt loom, walk-drive duty 20-50%, siesta
//! (scale 0.84) walk-drive > 3%") and from its `--simtest` pass condition. They
//! are asserted across several seeds, because the upstream draws its noise from
//! the system RNG: an invariant that only holds for one lucky seed is not an
//! invariant.

use gnat_sim::Circuit;
use gnat_sim::simtest::{self, Report};

/// Seeds are arbitrary but fixed, so a failure can be replayed exactly.
const SEEDS: [u64; 4] = [0x_F1_1E, 1, 20_240_701, 0xDEAD_BEEF];

fn circuit() -> Circuit {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/circuit.json");
    Circuit::load(path).expect("data/circuit.json should be committed alongside the code")
}

fn reports() -> Vec<Report> {
    SEEDS.iter().map(|&s| simtest::run(circuit(), s)).collect()
}

#[test]
fn the_circuit_is_the_one_the_etl_produced() {
    let c = circuit();
    assert_eq!(c.len(), 668, "neuron count changed");
    assert_eq!(c.edges.len(), 18_968, "edge count changed");

    let sim = gnat_sim::Lif::new(c, 0);
    let g = sim.groups();
    // The two giant fibres are DNp01 left and right, and the whole escape
    // behaviour hangs off them.
    assert_eq!(g.gf.len(), 2, "giant fibre population");
    assert_eq!(g.dna_l.len(), 2, "left steering DNs");
    assert_eq!(g.dna_r.len(), 2, "right steering DNs");
    assert_eq!(g.fwd.len(), 2, "DNp09 forward-walking command");
    assert!(
        g.loom_left.len() > 100 && g.loom_right.len() > 100,
        "loom populations"
    );
    assert!(
        !g.ascend.is_empty() && !g.sens.is_empty(),
        "feedback partners"
    );
}

#[test]
fn upstream_pass_condition_holds_on_every_seed() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert!(r.passed(), "seed {seed:#x} failed:\n{r}");
    }
}

/// INVARIANT: the giant fibre is silent over 4 s of rest. A fly whose escape
/// command fires spontaneously never stops taking off.
#[test]
fn gf_is_silent_at_rest() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert_eq!(r.gf_spontaneous, 0, "seed {seed:#x} fired GF at rest");
    }
}

/// INVARIANT: an abrupt loom drives the giant fibre within about 10 ms.
///
/// Escape is a race: the LC->GF electrical drive carries a x6 gap-junction
/// boost, against roughly 1,200 synapses of feedforward inhibition delayed by
/// 4 ms. The window between those two is the whole mechanism.
#[test]
fn abrupt_loom_fires_gf_inside_the_inhibition_window() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert!(r.gf_on_loom > 0, "seed {seed:#x}: no GF spike on loom");
        assert!(
            (0..=10).contains(&r.gf_latency_ms),
            "seed {seed:#x}: GF latency {} ms is outside the documented window",
            r.gf_latency_ms
        );
    }
}

/// INVARIANT: walk drive is on 20-50% of the time. Outside that band the fly
/// either never moves or never stops.
#[test]
fn walk_drive_duty_stays_in_band() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert!(
            (20.0..=50.0).contains(&r.walk_on_pct),
            "seed {seed:#x}: walk duty {:.0}% outside 20-50%",
            r.walk_on_pct
        );
        // Fluctuating, not latched: a flat rate means the command neuron is
        // saturated or dead.
        assert!(
            r.fwd_max_hz - r.fwd_min_hz > 1.0,
            "seed {seed:#x}: DNp09 rate barely moved ({:.1}-{:.1} Hz)",
            r.fwd_min_hz,
            r.fwd_max_hz
        );
    }
}

/// INVARIANT: the midday siesta slows the fly without paralysing it.
///
/// The operating point is razor-thin — neurons rest at `baseline * 20.4`
/// against a threshold of 1.0 — so scaling baselines linearly silences whole
/// populations. The scale is compressed toward 1 instead, and this is the test
/// that catches a regression to the "siesta coma".
#[test]
fn the_siesta_does_not_paralyse_the_fly() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert!(
            r.siesta_walk_on_pct > 3.0,
            "seed {seed:#x}: siesta walk duty {:.0}% — the fly is comatose",
            r.siesta_walk_on_pct
        );
        // Deliberately no "siesta < waking day" assertion: phase 3 runs with
        // gait proprioception feeding the ascending neurons and phase 3b does
        // not, so the two duties are not measuring the same thing. The
        // upstream bound is a floor, and a floor is what belongs here.
    }
}

/// INVARIANT: click stimulation reaches the circuit, which is what makes the
/// brain window interactive rather than decorative.
#[test]
fn click_stimulation_lands() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert!(
            r.gf_stim_fired,
            "seed {seed:#x}: stimulating GF did not fire it"
        );
        assert!(
            r.groom_stim_hz > 10.0,
            "seed {seed:#x}: stimulating DNg11 gave only {:.0} Hz",
            r.groom_stim_hz
        );
    }
}

/// INVARIANT: the wind pathway can also trigger escape. Sensory partners reach
/// the giant fibre through the same boosted electrical coupling as the loom
/// detectors.
#[test]
fn an_air_puff_can_trigger_escape() {
    for (seed, r) in SEEDS.iter().zip(reports()) {
        assert!(
            r.gf_on_puff > 0,
            "seed {seed:#x}: air puff never reached GF"
        );
    }
}

/// INVARIANT: the simulation is deterministic given a seed. Without this a
/// failing run cannot be replayed, and any desync must be assumed to be in the
/// sense layer.
#[test]
fn is_deterministic_for_a_seed() {
    let a = simtest::run(circuit(), 12_345);
    let b = simtest::run(circuit(), 12_345);
    assert_eq!(a.gf_latency_ms, b.gf_latency_ms);
    assert_eq!(a.gf_on_loom, b.gf_on_loom);
    assert_eq!(a.gf_on_puff, b.gf_on_puff);
    assert_eq!(a.walk_on_pct, b.walk_on_pct);
    assert_eq!(a.spont_pop_hz, b.spont_pop_hz);
}
