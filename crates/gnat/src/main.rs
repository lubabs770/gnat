//! Entry point. Wires the desktop senses into the sim and the sim into the
//! renderer.

mod app;
mod coord;
mod overlay_test;
mod png;
mod render;
mod snapshot;

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--senses") => senses(),
        Some("--run") => run(),
        Some("--snapshot") => shot(&args),
        Some("--simtest") => simtest(),
        Some("--behaviortest") => behaviortest(),
        Some("--overlay-test") => overlay_test::run(true),
        Some("--overlay-test-control") => overlay_test::run(false),
        Some("--version") | Some("-V") => {
            println!("gnat {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown argument: {other}");
            usage();
            std::process::exit(2);
        }
        None => run(),
    }
}

fn usage() {
    eprintln!(
        "gnat {}\n\
         \n\
         usage:\n\
         \x20 gnat            put a fly on the screen (same as --run)\n\
         \x20 gnat --snapshot <file.png> [seconds]   headless render, plus a zoomed crop\n\
         \x20 gnat --senses    dump one reading from every desktop sense\n\
         \x20 gnat --simtest   headless circuit test; exits non-zero on failure\n\
         \x20 gnat --behaviortest          stimulate neurons, watch the body react\n\
         \x20 gnat --overlay-test          prove the overlay passes clicks through\n\
         \x20 gnat --overlay-test-control  the same, with an input region, as a control\n\
         \n\
         the brain view and the control surface are not wired up yet (see README).",
        env!("CARGO_PKG_VERSION")
    );
}

/// Where the connectome lives, relative to the repository root.
const CIRCUIT: &str = "data/circuit.json";

/// A fixed seed, so a failure is reproducible. The original draws from the
/// system RNG and cannot be replayed.
const SEED: u64 = 0x_F1_1E;

fn run() -> Result<()> {
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    app::run(circuit, SEED)
}

fn shot(args: &[String]) -> Result<()> {
    let path = args
        .get(1)
        .context("usage: gnat --snapshot <file.png> [seconds]")?;
    let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    snapshot::run(circuit, SEED, std::path::Path::new(path), seconds)
}

fn simtest() -> Result<()> {
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    let report = gnat_sim::simtest::run(circuit, SEED);
    println!("{report}");
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}

fn behaviortest() -> Result<()> {
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    let report = gnat_body::behaviortest::run(&circuit, SEED);
    println!("{report}");
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}

fn senses() -> Result<()> {
    let hypr = gnat_senses::Hypr::connect()?;
    let clients = hypr.clients()?;
    let cursor = hypr.cursor_pos()?;
    let thermal = gnat_senses::Thermal::discover();

    println!("windows  {}", clients.len());
    println!("cursor   {},{}", cursor.x, cursor.y);
    println!("hottest  {:?} C", thermal.hottest_c());
    Ok(())
}
