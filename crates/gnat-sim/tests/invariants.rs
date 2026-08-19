//! Behavioural invariants the port must reproduce.
//!
//! The numbers marked PENDING are placeholders until the original Sim.swift
//! `--simtest` thresholds are transcribed; the test bodies are written so that
//! only the constant changes, not the structure.

use gnat_sim::connectome::{Connectome, Neuron, Role};
use gnat_sim::{LifParams, RateProbe, Sim};

/// A hand-built chain: sensory -> 3 interneurons -> motor, one synapse each.
/// Enough to exercise delay accumulation and the reset/refractory path.
fn chain(len: usize, weight: f32) -> Connectome {
    let neurons: Vec<Neuron> = (0..len)
        .map(|i| Neuron {
            root_id: i as u64,
            role: if i == 0 {
                Role::Sensory
            } else if i + 1 == len {
                Role::Motor
            } else {
                Role::Interneuron
            },
            cell_type: if i == 0 {
                1
            } else if i + 1 == len {
                2
            } else {
                0
            },
            pos: [i as f32, 0.0, 0.0],
        })
        .collect();

    // Each neuron but the last has exactly one outgoing edge.
    let offsets: Vec<u32> = (0..=len).map(|i| i.min(len - 1) as u32).collect();
    let targets: Vec<u32> = (1..len).map(|i| i as u32).collect();
    let weights = vec![weight; len - 1];

    Connectome {
        neurons,
        offsets,
        targets,
        weights,
        type_names: vec!["inter".into(), "sensory".into(), "motor".into()],
    }
}

/// A single synapse strong enough to relay a spike on its own. Real neurons
/// need convergent input to fire; these synthetic chains are one synapse wide,
/// so the weight stands in for that whole fan-in.
const SUPRA_W: f32 = 40.0;
/// Injected current that reliably drives the first neuron of a chain.
const SUPRA_I: f32 = 5.0;

fn params() -> LifParams {
    LifParams {
        w_scale: 1.0,
        ..LifParams::default()
    }
}

#[test]
fn connectome_validates() {
    chain(5, 2.0).validate().unwrap();
}

#[test]
fn round_trips_through_the_gnat_format() {
    let c = chain(6, 1.5);
    let mut buf = Vec::new();
    c.write_to(&mut buf).unwrap();
    let back = Connectome::read_from(&mut buf.as_slice()).unwrap();

    assert_eq!(back.neuron_count(), c.neuron_count());
    assert_eq!(back.synapse_count(), c.synapse_count());
    assert_eq!(back.offsets, c.offsets);
    assert_eq!(back.targets, c.targets);
    assert_eq!(back.weights, c.weights);
    assert_eq!(back.type_names, c.type_names);
    assert_eq!(back.neurons[0].role, Role::Sensory);
}

/// INVARIANT: at rest, with no sensory drive, the giant fibre stays silent.
/// A brain that self-ignites has the wrong weight scale and every downstream
/// behaviour is noise.
#[test]
fn silent_at_rest() {
    let mut sim = Sim::new(chain(8, 2.0), params());
    for _ in 0..5_000 {
        sim.step();
        assert!(
            sim.spikes().is_empty(),
            "spontaneous spike at t={}",
            sim.time()
        );
    }
}

/// INVARIANT: a suprathreshold stimulus propagates the whole chain, and the
/// motor end fires. This is the structural half of the loom-to-escape latency
/// check; the timing half is below.
#[test]
fn stimulus_reaches_the_motor_end() {
    let c = chain(5, SUPRA_W);
    let motor: Vec<u32> = c.by_type("motor");
    assert_eq!(motor.len(), 1);

    let p = params();
    let mut sim = Sim::new(c, p);
    let mut probe = RateProbe::new(motor, 100, p.dt);

    for _ in 0..200 {
        sim.inject(0, SUPRA_I);
        sim.step();
        probe.observe(&sim);
    }
    assert!(
        probe.active(),
        "motor population never fired under sustained drive"
    );
}

/// INVARIANT: escape latency. Time from stimulus onset to the first motor spike
/// must land in the documented window.
///
/// PENDING: the real bound is 4 ms through the actual loom pathway. With the
/// placeholder chain the assertion is on the mechanism (latency grows with path
/// length, and is bounded) rather than on the biological figure.
#[test]
fn escape_latency_is_bounded_and_path_dependent() {
    let p = params();

    let latency = |len: usize| -> u64 {
        let c = chain(len, SUPRA_W);
        let motor = c.by_type("motor")[0];
        let mut sim = Sim::new(c, p);
        for _ in 0..1_000 {
            sim.inject(0, SUPRA_I);
            sim.step();
            if sim.spikes().contains(&motor) {
                return sim.tick_count();
            }
        }
        u64::MAX
    };

    let short = latency(3);
    let long = latency(9);

    assert!(short < u64::MAX, "short chain never reached motor");
    assert!(long < u64::MAX, "long chain never reached motor");
    assert!(
        long > short,
        "latency did not grow with path length: {long} vs {short}"
    );
    assert!(long < 100, "latency {long} ticks is implausibly slow");
}

/// INVARIANT: the refractory period caps the firing rate. Without this the
/// brain view is a strobe and the gait model saturates.
#[test]
fn refractory_period_caps_firing_rate() {
    let p = params();
    let c = chain(2, 0.0);
    let mut sim = Sim::new(c, p);

    let mut spikes = 0;
    for _ in 0..1_000 {
        sim.inject(0, 100.0);
        sim.step();
        spikes += sim.spikes().iter().filter(|&&s| s == 0).count();
    }

    let max_hz = 1.0 / (p.t_refrac + p.dt);
    assert!(
        (spikes as f32) <= max_hz * 1.05,
        "{spikes} Hz exceeds the refractory ceiling of {max_hz} Hz"
    );
}

/// INVARIANT: the sim is deterministic. Two runs from the same state and the
/// same input produce identical spike trains, so a desync is always a bug in
/// the sense layer rather than in the brain.
#[test]
fn is_deterministic() {
    let record = || {
        let mut sim = Sim::new(chain(6, 3.0), params());
        let mut trace = Vec::new();
        for t in 0..500 {
            if t % 37 == 0 {
                sim.inject(0, 3.0);
            }
            sim.step();
            trace.push(sim.spikes().to_vec());
        }
        trace
    };
    assert_eq!(record(), record());
}
