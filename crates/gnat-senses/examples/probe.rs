//! Live smoke test of every sense against the running compositor.
//!
//!     cargo run -p gnat-senses --example probe

use anyhow::Result;
use gnat_senses::{Hypr, Thermal, circadian, ledges, terrain::CursorTracker};
use std::time::{Duration, Instant, SystemTime};

fn main() -> Result<()> {
    let hypr = Hypr::connect()?;

    let monitors = hypr.monitors()?;
    println!("monitors:");
    for m in &monitors {
        println!(
            "  {} {}x{} @{} +{},{} scale {}",
            m.name, m.width, m.height, m.refresh_rate, m.x, m.y, m.scale
        );
    }
    let (sw, sh) = monitors
        .first()
        .map(|m| (m.x + m.width, m.y + m.height))
        .unwrap_or((1920, 1080));

    let clients = hypr.clients()?;
    println!("\nwindows: {}", clients.len());
    for c in &clients {
        println!(
            "  {:<14} {:>5},{:<5} {:>5}x{:<5} {}",
            c.class,
            c.at.0,
            c.at.1,
            c.size.0,
            c.size.1,
            c.title.chars().take(48).collect::<String>()
        );
    }

    let l = ledges(&clients, sw, sh);
    println!("\nledges: {}", l.len());
    for ledge in l.iter().take(12) {
        println!(
            "  y={:<5} x {}..{} ({}px)",
            ledge.y,
            ledge.x0,
            ledge.x1,
            ledge.width()
        );
    }

    let thermal = Thermal::discover();
    println!("\nthermal sensors: {}", thermal.sensors().len());
    for s in thermal.sensors().iter().take(8) {
        println!("  {}", s.label);
    }
    println!(
        "  hottest: {:?} C  arousal: {:.2}",
        thermal.hottest_c(),
        thermal.arousal(40.0, 85.0)
    );

    let hour = circadian::local_hour(SystemTime::now(), 0);
    println!("\nclock: {hour:.2}h UTC (the activity curve itself lives in gnat-body)");

    // Cursor polling: the one sense that has to be sampled rather than pushed.
    println!("\npolling the cursor at 60 Hz for 2s...");
    let mut tracker = CursorTracker::new();
    let start = Instant::now();
    let mut polls = 0u32;
    let mut peak = 0.0f32;
    while start.elapsed() < Duration::from_secs(2) {
        let pos = tracker.update(hypr.cursor_pos()?);
        polls += 1;
        let (vx, vy) = tracker.velocity();
        peak = peak.max((vx * vx + vy * vy).sqrt());
        if polls.is_multiple_of(30) {
            println!(
                "  at {},{}  velocity {:.0},{:.0} px/s",
                pos.x, pos.y, vx, vy
            );
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    println!(
        "  {polls} polls in 2s ({:.1} Hz), peak speed {peak:.0} px/s",
        polls as f32 / 2.0
    );

    println!("\nevent stream (10s, move or open a window):");
    let stream = gnat_senses::EventStream::connect()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    for event in stream {
        println!("  {event:?}");
        if Instant::now() > deadline {
            break;
        }
    }
    Ok(())
}
