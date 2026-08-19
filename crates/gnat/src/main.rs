//! Entry point. Wires the desktop senses into the sim and the sim into the
//! renderer.

mod overlay_test;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--senses") => senses(),
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
        None => {
            usage();
            Ok(())
        }
    }
}

fn usage() {
    eprintln!(
        "gnat {}\n\
         \n\
         usage:\n\
         \x20 gnat --senses    dump one reading from every desktop sense\n\
         \x20 gnat --simtest   headless circuit test; exits non-zero on failure\n\
         \x20 gnat --behaviortest          stimulate neurons, watch the body react\n\
         \x20 gnat --overlay-test          prove the overlay passes clicks through\n\
         \x20 gnat --overlay-test-control  the same, with an input region, as a control\n\
         \n\
         not wired up yet: the overlay and the fly body (see README).",
        env!("CARGO_PKG_VERSION")
    );
}

/// Where the connectome lives, relative to the repository root.
const CIRCUIT: &str = "data/circuit.json";

/// A fixed seed, so a failure is reproducible. The original draws from the
/// system RNG and cannot be replayed.
const SEED: u64 = 0x_F1_1E;

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
