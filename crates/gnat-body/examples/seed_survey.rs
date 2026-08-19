//! How often does each behaviour check pass, across many seeds?
//!
//!     cargo run --release -p gnat-body --example seed_survey [seeds]
//!
//! The scenarios drive a noisy network, so some spread is expected and the
//! question is *how much*. A check that passes on 19 seeds out of 20 is
//! measuring biology; one that passes on 11 is measuring luck, and wants
//! looking at.

use gnat_body::behaviortest;
use gnat_sim::Circuit;
use std::collections::BTreeMap;

fn main() {
    let seeds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/circuit.json");
    let circuit = Circuit::load(path).expect("data/circuit.json");

    let mut passes: BTreeMap<&'static str, u64> = BTreeMap::new();
    for seed in 0..seeds {
        for o in behaviortest::run(&circuit, seed).outcomes {
            *passes.entry(o.name).or_default() += o.passed as u64;
        }
    }

    let mut rows: Vec<_> = passes.into_iter().collect();
    rows.sort_by_key(|(_, n)| *n);
    for (name, n) in rows {
        let pct = 100.0 * n as f64 / seeds as f64;
        println!("{pct:5.0}%  {n:>3}/{seeds}  {name}");
    }
}
