//! The behaviour suite, run against the real circuit.
//!
//! The scenarios drive a noisy network and the body itself wanders randomly, so
//! three of the seventeen checks are genuinely stochastic. The original has the
//! same property and simply cannot see it: it draws a fresh system-random seed
//! on every run, so a flaky check there looks like an occasional mystery
//! failure. Seeding lets us measure the flakiness instead of tripping over it.
//!
//! So this asserts two things: the shipped seed is green, and every check is
//! reliable across a sample of seeds. A regression drops a check to near zero,
//! which either assertion catches.

use gnat_body::behaviortest;
use gnat_sim::Circuit;
use std::collections::BTreeMap;

/// The seed `gnat --behaviortest` uses.
const DEFAULT_SEED: u64 = 0x_F1_1E;

/// Measured pass rates over 40 seeds, as of the port:
///
/// | check | rate |
/// |---|---|
/// | ledge attach + follow window edge | 85% |
/// | DNp09 stim -> walks, speed rises | 92% |
/// | DNa-left stim -> left turn | 98% |
/// | the other fourteen | 100% |
///
/// The bar sits below the lowest of those with room to spare, so ordinary
/// noise does not fail the build but a broken check still does.
const MIN_PASS_RATE: f64 = 0.75;
const SURVEY_SEEDS: u64 = 24;

fn circuit() -> Circuit {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/circuit.json");
    Circuit::load(path).expect("data/circuit.json should be committed alongside the code")
}

#[test]
fn all_seventeen_checks_pass_on_the_shipped_seed() {
    let report = behaviortest::run(&circuit(), DEFAULT_SEED);
    assert_eq!(report.outcomes.len(), 17, "the suite lost a check");
    assert!(report.passed(), "{report}");
}

#[test]
fn every_check_is_reliable_across_seeds() {
    let circuit = circuit();
    let mut passes: BTreeMap<&'static str, u64> = BTreeMap::new();
    for seed in 0..SURVEY_SEEDS {
        for o in behaviortest::run(&circuit, seed).outcomes {
            *passes.entry(o.name).or_default() += o.passed as u64;
        }
    }

    let mut unreliable: Vec<String> = Vec::new();
    for (name, n) in &passes {
        let rate = *n as f64 / SURVEY_SEEDS as f64;
        if rate < MIN_PASS_RATE {
            unreliable.push(format!(
                "  {:.0}%  {n}/{SURVEY_SEEDS}  {name}",
                rate * 100.0
            ));
        }
    }
    assert!(
        unreliable.is_empty(),
        "checks below {:.0}%:\n{}",
        MIN_PASS_RATE * 100.0,
        unreliable.join("\n")
    );
    assert_eq!(passes.len(), 17);
}

#[test]
fn the_suite_is_deterministic_for_a_seed() {
    let circuit = circuit();
    let details = |seed| {
        behaviortest::run(&circuit, seed)
            .outcomes
            .into_iter()
            .map(|o| (o.name, o.passed, o.detail))
            .collect::<Vec<_>>()
    };
    assert_eq!(details(999), details(999));
}
