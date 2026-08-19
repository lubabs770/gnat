//! Entry point. Wires the desktop senses into the sim and the sim into the
//! renderer.

mod app;
mod brain;
mod control;
mod coord;
mod font;
mod overlay_test;
mod png;
mod render;
mod snapshot;

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--senses") => senses(),
        Some("pause") | Some("resume") | Some("toggle") | Some("scare") | Some("quit")
        | Some("status") | Some("add") | Some("remove") | Some("brain") => {
            println!("{}", control::send(args[0].as_str())?);
            Ok(())
        }
        Some("waybar") => {
            println!("{}", control::waybar());
            Ok(())
        }
        Some("--run") => run(&args),
        Some("--brain") => run(&args),
        Some("--output") => run(&args),
        Some("outputs") => outputs(),
        Some("--snapshot") => shot(&args),
        Some("--brainshot") => brainshot(&args),
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
        None => run(&args),
    }
}

fn usage() {
    eprintln!(
        "gnat {}\n\
         \n\
         usage:\n\
         \x20 gnat            put a fly on the screen (same as --run)\n\
         \x20 gnat --brain    the same, plus the brain window\n\
         \x20 gnat --output <NAME>       pin the overlay to one output (see: gnat outputs)\n\
         \n\
         \x20 gnat pause | resume | toggle | scare | quit | status\n\
         \x20 gnat add | remove          add or remove a brainless extra fly\n\
         \x20 gnat brain                 open the brain window on a running fly\n\
         \x20 gnat outputs               list outputs\n\
         \x20 gnat waybar     one line of JSON for a Waybar custom module\n\
         \x20 gnat --snapshot <file.png> [seconds]   headless render, plus a zoomed crop\n\
         \x20 gnat --brainshot <file.png> [seconds]  headless render of the brain view\n\
         \x20 gnat --senses    dump one reading from every desktop sense\n\
         \x20 gnat --simtest   headless circuit test; exits non-zero on failure\n\
         \x20 gnat --behaviortest          stimulate neurons, watch the body react\n\
         \x20 gnat --overlay-test          prove the overlay passes clicks through\n\
         \x20 gnat --overlay-test-control  the same, with an input region, as a control\n\
         \n\
         the brain view needs --brain; see README for what is still missing.",
        env!("CARGO_PKG_VERSION")
    );
}

/// Where the connectome lives, relative to the repository root.
const CIRCUIT: &str = "data/circuit.json";
/// The brain view's point cloud. Only loaded when the view is asked for.
const POINTS: &str = "data/brain_points.json";

/// A fixed seed, so a failure is reproducible. The original draws from the
/// system RNG and cannot be replayed.
const SEED: u64 = 0x_F1_1E;

fn run(args: &[String]) -> Result<()> {
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    // `--output NAME` may appear on its own or after `--run` / `--brain`.
    let output = args
        .iter()
        .position(|a| a == "--output")
        .and_then(|i| args.get(i + 1))
        .cloned();
    app::run(
        circuit,
        SEED,
        app::Options {
            brain: args.iter().any(|a| a == "--brain"),
            output,
            points_path: Some(POINTS.into()),
        },
    )
}

/// List the outputs `--output` will accept.
fn outputs() -> Result<()> {
    for m in gnat_senses::Hypr::connect()?.monitors()? {
        println!(
            "{:<12} {}x{} @{:.0} at {},{} scale {}",
            m.name, m.width, m.height, m.refresh_rate, m.x, m.y, m.scale
        );
    }
    Ok(())
}

fn shot(args: &[String]) -> Result<()> {
    let path = args
        .get(1)
        .context("usage: gnat --snapshot <file.png> [seconds]")?;
    let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    snapshot::run(circuit, SEED, std::path::Path::new(path), seconds)
}

fn brainshot(args: &[String]) -> Result<()> {
    let path = args
        .get(1)
        .context("usage: gnat --brainshot <file.png> [seconds]")?;
    let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let circuit = gnat_sim::Circuit::load(CIRCUIT)?;
    let points = gnat_sim::BrainPoints::load(POINTS)?;
    snapshot::brainshot(circuit, points, SEED, std::path::Path::new(path), seconds)
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
