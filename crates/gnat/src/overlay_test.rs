//! Milestone 3's proof: does a `wlr-layer-shell` overlay really let clicks
//! through on Hyprland?
//!
//! Claiming it does is cheap. This measures it. The overlay spans the whole
//! output on the `overlay` layer, and a probe thread drives the real cursor
//! across it with `hyprctl dispatch movecursor` while the surface counts every
//! pointer event addressed to it. With an empty input region that count must
//! stay at zero; run with `--input` and the same sweep must produce a non-zero
//! count, which is the control that proves the test can fail.

use anyhow::{Context, Result};
use gnat_overlay::{Canvas, Config, Flow, Overlay, Rgba};
use gnat_senses::{Hypr, LayerLevel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Marks the extent of the surface, so the anchors are visible by eye too.
const EDGE: Rgba = Rgba::new(0xE0, 0x40, 0x40, 0x90);

/// Where the cursor is driven, as fractions of the output.
const SWEEP: [(f32, f32); 6] = [
    (0.5, 0.5),
    (0.2, 0.3),
    (0.8, 0.3),
    (0.8, 0.7),
    (0.2, 0.7),
    (0.5, 0.5),
];

pub fn run(click_through: bool) -> Result<()> {
    let hypr = Hypr::connect()?;
    let monitor = hypr
        .monitors()?
        .into_iter()
        .next()
        .context("no outputs reported by Hyprland")?;
    let restore = hypr.cursor_pos()?;

    println!(
        "overlay test on {} ({}x{}), click_through = {click_through}",
        monitor.name, monitor.width, monitor.height
    );

    let mut overlay = Overlay::new(Config {
        namespace: "gnat-overlay-test".into(),
        click_through,
        ..Config::default()
    })?;

    let stop = Arc::new(AtomicBool::new(false));
    let findings: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let swept = Arc::new(AtomicBool::new(false));
    let probe = {
        let stop = stop.clone();
        let swept = swept.clone();
        let findings = findings.clone();
        let (w, h) = (monitor.width, monitor.height);
        std::thread::spawn(move || {
            let hypr = Hypr::connect().expect("second IPC connection");
            let out = |s: String| findings.lock().unwrap().push(s);
            // Let the surface map and take its first frame.
            std::thread::sleep(Duration::from_millis(600));

            match layer_report(&hypr, "gnat-overlay-test") {
                Ok(line) => out(line),
                Err(e) => out(format!("layers:      FAILED to find the surface: {e}")),
            }

            let focus = |h: &Hypr| h.active_window().ok().flatten().map(|c| c.address);
            let focus_before = focus(&hypr);

            let mut moves = 0;
            for (fx, fy) in SWEEP {
                let (x, y) = ((fx * w as f32) as i32, (fy * h as f32) as i32);
                match hypr.move_cursor(x, y) {
                    Ok(()) => moves += 1,
                    Err(e) => {
                        out(format!("cursor:      move to {x},{y} failed: {e}"));
                        break;
                    }
                }
                // Long enough for the compositor to deliver any enter event
                // and for the client to dispatch it.
                std::thread::sleep(Duration::from_millis(150));
            }
            // The whole test is vacuous if the cursor never moved, so this is
            // recorded rather than assumed.
            let confirmed = hypr.cursor_pos().ok();
            out(format!(
                "cursor:      {moves}/{} warps accepted, ended at {}",
                SWEEP.len(),
                confirmed.map_or("unknown".into(), |p| format!("{},{}", p.x, p.y))
            ));
            if moves == SWEEP.len() {
                swept.store(true, Ordering::Relaxed);
            }

            let focus_after = focus(&hypr);
            // The overlay sets KeyboardInteractivity::None and is not a
            // toplevel, so it can never be the focused window. A change here
            // means the warps dragged focus across real windows under
            // follow_mouse, or a human touched the mouse mid-run - noted, but
            // not evidence against the overlay.
            out(match (&focus_before, &focus_after) {
                (a, b) if a == b => format!(
                    "focus:       {}  (unchanged)",
                    a.as_deref().unwrap_or("(none)")
                ),
                (a, b) => format!(
                    "focus:       {} -> {}  (moved under follow_mouse; the overlay cannot take focus)",
                    a.as_deref().unwrap_or("(none)"),
                    b.as_deref().unwrap_or("(none)")
                ),
            });

            std::thread::sleep(Duration::from_millis(250));
            stop.store(true, Ordering::Relaxed);
        })
    };

    // Draw a moving marker, so the overlay is also verifiable by eye.
    let (mw, mh) = (monitor.width, monitor.height);
    let stop_draw = stop.clone();
    overlay.run(move |c: &mut Canvas| {
        c.clear();
        let t = c.time_ms as f32 / 1000.0;
        let cx = (mw as f32 * (0.5 + 0.35 * (t * 1.1).cos())) as i32;
        let cy = (mh as f32 * (0.5 + 0.25 * (t * 1.7).sin())) as i32;
        c.disc(cx, cy, 26, Rgba::new(0x30, 0xE0, 0x80, 0xC0));
        // A thin frame, so the extent of the surface is visible.
        c.rect(0, 0, c.width as i32, 2, EDGE);
        c.rect(0, c.height as i32 - 2, c.width as i32, 2, EDGE);

        if stop_draw.load(Ordering::Relaxed) {
            Flow::Exit
        } else {
            Flow::Continue
        }
    })?;

    let _ = probe.join();
    let _ = hypr.move_cursor(restore.x, restore.y);

    let (w, h) = overlay.size();
    let enters = overlay.pointer_enters();
    let buttons = overlay.pointer_buttons();

    println!("surface:     {w}x{h} (whole output means the anchors took effect)");
    for line in findings.lock().unwrap().iter() {
        println!("{line}");
    }
    println!("pointer:     {enters} enters, {buttons} presses on the overlay surface");

    // Without a completed sweep the pointer never crossed the surface, and a
    // zero count would prove nothing at all.
    let swept = swept.load(Ordering::Relaxed);
    if !swept {
        println!("\nFAIL: the cursor sweep did not complete, so this run proves nothing.");
        std::process::exit(1);
    }

    let ok = if click_through {
        enters == 0 && buttons == 0
    } else {
        enters > 0
    };
    println!(
        "\n{}",
        match (click_through, ok) {
            (true, true) =>
                "PASS: empty input region — the cursor crossed the whole surface and it never saw a thing.",
            (true, false) =>
                "FAIL: the overlay received pointer input despite an empty input region.",
            (false, true) =>
                "PASS (control): with an input region the same sweep IS seen, so the test can fail.",
            (false, false) =>
                "FAIL (control): no pointer events even with an input region — the probe is broken, not the overlay.",
        }
    );
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

/// Confirm the surface really is on the `overlay` level and really does span
/// the output — anchoring to all four edges is what makes that happen.
fn layer_report(hypr: &Hypr, namespace: &str) -> Result<String> {
    let (monitor, level, surface) = hypr
        .layers()?
        .into_iter()
        .find(|(_, _, s)| s.namespace == namespace)
        .with_context(|| format!("no layer surface named {namespace}"))?;
    anyhow::ensure!(
        level == LayerLevel::Overlay,
        "surface is on the {level} layer, not overlay"
    );
    Ok(format!(
        "layers:      on {monitor}, level {level}, {}x{} at {},{}",
        surface.w, surface.h, surface.x, surface.y
    ))
}
