//! Headless rendering, for looking at the fly without a compositor in the way.
//!
//! The original ships `--snapshot`; this is the same idea, plus a zoomed crop,
//! because a fly is about 20 px across on a 1920x1080 output and is invisible
//! in a full-size frame.
//!
//! The canvas is transparent by design, so both images are composited over a
//! checkerboard — otherwise a correct render and an empty one look identical.

use anyhow::Result;
use gnat_overlay::Canvas;
use gnat_sim::{BrainPoints, Circuit, Lif};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::app::App;
use crate::brain::{BrainData, SpikeBus, StimQueue, View};
use crate::png;

/// Frame rate the headless loop pretends to run at.
const FPS: u32 = 60;
/// Half-width of the zoom crop, in output pixels, before upscaling.
const CROP: i32 = 90;
const ZOOM: u32 = 6;
const CHECKER: u32 = 16;

pub fn run(circuit: Circuit, seed: u64, path: &Path, seconds: f32) -> Result<()> {
    let mut app = App::new(circuit, seed)?;
    let frame = app.output();
    let (w, h) = (frame.width as u32, frame.height as u32);

    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let frames = (seconds * FPS as f32).round() as u32;

    for i in 0..frames.max(1) {
        let mut canvas = Canvas {
            width: w,
            height: h,
            pixels: &mut pixels,
            time_ms: (i * 1000 / FPS) as u64,
        };
        app.frame(&mut canvas);
        // Real time, so the senses see a real cursor and a real clock rather
        // than a loop running as fast as it can.
        std::thread::sleep(Duration::from_millis((1000 / FPS) as u64));
    }

    let rgba = composite(&pixels, w, h);
    png::write_rgba(path, w, h, &rgba)?;

    let (fx, fy) = {
        let fly = app.fly();
        frame.to_screen(fly.pos.0, fly.pos.1)
    };
    let zoom_path = with_suffix(path, "-zoom");
    let (zw, zh, zoomed) = crop_and_zoom(&rgba, w, h, fx as i32, fy as i32);
    png::write_rgba(&zoom_path, zw, zh, &zoomed)?;

    let terrain = app.terrain().len();
    let flies = app.fly_count();

    let fly = app.fly();
    println!("state    {:?}  ({flies} fly/flies)", fly.state);
    println!(
        "position {:.0},{:.0} on screen  (alt {:.2})",
        fx, fy, fly.alt
    );
    println!(
        "terrain  {terrain} ledges, standing on {}",
        match fly.ledge {
            Some(l) => format!("window {:#x}", l.id),
            None => "nothing".into(),
        }
    );
    for l in app.terrain().iter().take(6) {
        println!(
            "  ledge  y={:>6.0}  x {:>6.0}..{:<6.0}  window {:#x}",
            l.y, l.x0, l.x1, l.id
        );
    }
    println!("wrote    {}", path.display());
    println!("wrote    {}  ({zw}x{zh}, {ZOOM}x)", zoom_path.display());
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map_or("png".into(), |e| e.to_string_lossy());
    path.with_file_name(format!("{stem}{suffix}.{ext}"))
}

/// Premultiplied BGRA over a checkerboard, into straight RGBA.
fn composite(pixels: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (b, g, r, a) = (
                pixels[i] as u32,
                pixels[i + 1] as u32,
                pixels[i + 2] as u32,
                pixels[i + 3] as u32,
            );
            let bg = if ((x / CHECKER) + (y / CHECKER)).is_multiple_of(2) {
                48
            } else {
                64
            };
            // The canvas is premultiplied, so the source term is already scaled.
            let mix = |c: u32| (c + bg * (255 - a) / 255).min(255) as u8;
            out[i] = mix(r);
            out[i + 1] = mix(g);
            out[i + 2] = mix(b);
            out[i + 3] = 255;
        }
    }
    out
}

/// A `CROP`-radius square around `(cx, cy)`, nearest-neighbour upscaled.
fn crop_and_zoom(rgba: &[u8], w: u32, h: u32, cx: i32, cy: i32) -> (u32, u32, Vec<u8>) {
    let side = (CROP * 2) as u32;
    let out_side = side * ZOOM;
    let mut out = vec![0u8; (out_side * out_side * 4) as usize];

    for oy in 0..out_side {
        for ox in 0..out_side {
            let sx = cx - CROP + (ox / ZOOM) as i32;
            let sy = cy - CROP + (oy / ZOOM) as i32;
            let o = ((oy * out_side + ox) * 4) as usize;
            if sx < 0 || sy < 0 || sx >= w as i32 || sy >= h as i32 {
                out[o + 3] = 255;
                continue;
            }
            let s = ((sy as u32 * w + sx as u32) * 4) as usize;
            out[o..o + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    (out_side, out_side, out)
}

/// Render the brain view headlessly, with a real loom driving real spikes.
///
/// The original has `--brainshot` for the same reason: a point cloud is exactly
/// the kind of thing that looks fine in code and wrong on screen.
pub fn brainshot(
    circuit: Circuit,
    points: BrainPoints,
    seed: u64,
    path: &Path,
    seconds: f32,
) -> Result<()> {
    let (w, h) = (760u32, 620u32);
    let data = std::sync::Arc::new(BrainData::new(&circuit, points));
    let bus = std::sync::Arc::new(SpikeBus::default());
    let stims = std::sync::Arc::new(StimQueue::default());

    let mut sim = Lif::new(circuit, seed);
    sim.set_spike_logging(true);
    // Settle, then drive an abrupt loom so the shot has a giant-fibre volley in
    // it rather than only baseline crackle.
    sim.step(400);
    let _ = sim.consume_gf();

    let frames = (seconds * 60.0).round() as u32;
    let bus_for_sim = bus.clone();
    let mut view = View::new(data.clone(), bus, stims);
    let rgba = crate::brain::render_offscreen(&mut view, w, h, frames, |i| {
        if i > frames / 3 {
            sim.inputs.loom_l = 1.0;
            sim.inputs.loom_r = 0.5;
        }
        sim.step(16);
        bus_for_sim.push(&sim.drain_spikes());
    });

    // The brain view is opaque, so it only needs the alpha flattened.
    let mut out = vec![0u8; rgba.len()];
    for (o, p) in out.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        o.copy_from_slice(&[p[2], p[1], p[0], 255]);
    }
    png::write_rgba(path, w, h, &out)?;
    println!(
        "neurons  {} simulated, {} in the cloud",
        data.neurons.len(),
        data.cloud.len()
    );
    println!("wrote    {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_suffix_keeps_the_extension() {
        let p = with_suffix(Path::new("/tmp/shot.png"), "-zoom");
        assert_eq!(p, Path::new("/tmp/shot-zoom.png"));
    }

    #[test]
    fn compositing_makes_everything_opaque() {
        // Wider than one checker square, or only a single shade shows up.
        const N: u32 = CHECKER * 3;
        let pixels = vec![0u8; (N * N * 4) as usize];
        let out = composite(&pixels, N, N);
        assert!(out.chunks_exact(4).all(|p| p[3] == 255));
        // A fully transparent canvas must show the checkerboard, not black.
        assert!(out.chunks_exact(4).any(|p| p[0] == 48));
        assert!(out.chunks_exact(4).any(|p| p[0] == 64));
    }

    #[test]
    fn cropping_off_the_edge_pads_rather_than_panicking() {
        let rgba = vec![200u8; (32 * 32 * 4) as usize];
        let (w, h, out) = crop_and_zoom(&rgba, 32, 32, 0, 0);
        assert_eq!((w, h), (CROP as u32 * 2 * ZOOM, CROP as u32 * 2 * ZOOM));
        assert_eq!(out.len(), (w * h * 4) as usize);
    }
}
