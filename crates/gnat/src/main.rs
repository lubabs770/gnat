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
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--senses") => senses(),
        Some("pause") | Some("resume") | Some("toggle") | Some("scare") | Some("quit")
        | Some("status") | Some("add") | Some("remove") | Some("brain") => {
            println!("{}", control::send(args[0].as_str())?);
            Ok(())
        }
        // `flies` takes an optional count, so it is sent as a whole line.
        Some("flies") => {
            println!("{}", control::send(&args.join(" "))?);
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
         \x20 gnat --flies <N>           start with N flies (the first has the brain)\n\
         \x20 gnat --output <NAME>       pin the overlay to one output (see: gnat outputs)\n\
         \n\
         \x20 gnat pause | resume | toggle | scare | quit | status\n\
         \x20 gnat flies [N]             set the number of flies, or report it\n\
         \x20 gnat add | remove          add or remove one brainless extra fly\n\
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

/// Find the directory holding `circuit.json` and `brain_points.json`.
///
/// The binary is normally run from a symlink on `PATH`, so the data cannot be
/// looked up relative to the working directory alone. Searched in order:
///
/// 1. `$GNAT_DATA`, for anyone who keeps it somewhere else entirely;
/// 2. `<exe>/data`, the layout an installed copy would use;
/// 3. `./data`, for running out of a checkout;
/// 4. upwards from the executable, which finds the repository root from
///    `target/release/gnat`;
/// 5. `~/gnat/data`, the conventional checkout.
fn data_dir() -> Result<PathBuf> {
    resolve_data_dir(std::env::var("GNAT_DATA").ok().as_deref())
}

/// The search itself, with the override passed in rather than read from the
/// environment — mutating process-global state from a test would race every
/// other test in the binary.
fn resolve_data_dir(override_dir: Option<&str>) -> Result<PathBuf> {
    // An explicit override is authoritative: if it is wrong, say so rather than
    // quietly using something else and leaving the user to wonder which data
    // they are looking at.
    if let Some(dir) = override_dir {
        let dir = PathBuf::from(dir);
        anyhow::ensure!(
            dir.join("circuit.json").is_file(),
            "GNAT_DATA is set to {}, which has no circuit.json",
            dir.display()
        );
        return Ok(dir);
    }

    let mut tried: Vec<PathBuf> = Vec::new();
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        // Resolve the symlink first, or `~/.local/bin/gnat` would search there.
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("data"));
            for ancestor in dir.ancestors().take(4) {
                candidates.push(ancestor.join("data"));
            }
        }
    }
    candidates.push(PathBuf::from("data"));
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(home).join("gnat/data"));
    }

    for dir in candidates {
        if dir.join("circuit.json").is_file() {
            return Ok(dir);
        }
        tried.push(dir);
    }
    anyhow::bail!(
        "cannot find circuit.json. Looked in:\n  {}\nSet GNAT_DATA to the directory holding it.",
        tried
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    )
}

fn circuit_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("circuit.json"))
}

fn points_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("brain_points.json"))
}

/// A fixed seed, so a failure is reproducible. The original draws from the
/// system RNG and cannot be replayed.
const SEED: u64 = 0x_F1_1E;

fn run(args: &[String]) -> Result<()> {
    let circuit = gnat_sim::Circuit::load(circuit_path()?)?;
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
            flies: args
                .iter()
                .position(|a| a == "--flies")
                .and_then(|i| args.get(i + 1))
                .and_then(|n| n.parse().ok())
                .unwrap_or(1),
            output,
            points_path: Some(points_path()?),
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
    let circuit = gnat_sim::Circuit::load(circuit_path()?)?;
    snapshot::run(circuit, SEED, std::path::Path::new(path), seconds)
}

fn brainshot(args: &[String]) -> Result<()> {
    let path = args
        .get(1)
        .context("usage: gnat --brainshot <file.png> [seconds]")?;
    let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let circuit = gnat_sim::Circuit::load(circuit_path()?)?;
    let points = gnat_sim::BrainPoints::load(points_path()?)?;
    snapshot::brainshot(circuit, points, SEED, std::path::Path::new(path), seconds)
}

fn simtest() -> Result<()> {
    let circuit = gnat_sim::Circuit::load(circuit_path()?)?;
    let report = gnat_sim::simtest::run(circuit, SEED);
    println!("{report}");
    if !report.passed() {
        std::process::exit(1);
    }
    Ok(())
}

fn behaviortest() -> Result<()> {
    let circuit = gnat_sim::Circuit::load(circuit_path()?)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The binary normally runs from a symlink on PATH, so this must not
    /// depend on the working directory.
    #[test]
    fn the_data_directory_is_found_from_anywhere() {
        let dir =
            resolve_data_dir(None).expect("data/ should be discoverable from the test binary");
        assert!(dir.join("circuit.json").is_file(), "{}", dir.display());
        assert!(points_path().unwrap().is_file());
    }

    #[test]
    fn an_explicit_override_that_is_wrong_is_an_error_not_a_fallback() {
        let err = resolve_data_dir(Some("/nonexistent-gnat-data"))
            .expect_err("a bad override must not fall through");
        let message = err.to_string();
        assert!(message.contains("GNAT_DATA"), "{message}");
        assert!(message.contains("/nonexistent-gnat-data"), "{message}");
    }

    #[test]
    fn a_good_override_wins() {
        let found = data_dir().unwrap();
        let via_override = resolve_data_dir(Some(found.to_str().unwrap())).unwrap();
        assert_eq!(via_override, found);
    }
}
