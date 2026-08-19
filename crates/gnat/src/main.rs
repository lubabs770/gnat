//! Entry point. Wires the desktop senses into the sim and the sim into the
//! renderer.

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--senses") => senses(),
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
         \n\
         not wired up yet: the overlay, and the sim itself (see README).",
        env!("CARGO_PKG_VERSION")
    );
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
