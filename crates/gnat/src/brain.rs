//! The brain view: 23,210 real somas, the 668 simulated neurons drawn brighter
//! on top, and a flash wherever something spikes. Click it to stimulate.
//!
//! Software 3D. The point cloud is accumulated additively into a float buffer
//! and tone-mapped at the end, which needs no depth sorting — order-independent
//! by construction, and it gives the glow that a point cloud wants anyway.
//!
//! It runs on its own thread and talks to the sim through two small channels:
//! spikes come out on a [`SpikeBus`], stimulation goes in on a [`StimQueue`].
//! That is the original's arrangement too; its brain panel renders on the
//! AppKit thread while the sim advances in the fly's render loop.

use anyhow::Result;
use gnat_overlay::{Canvas, Click, Flow, Rgba, Window, WindowConfig};
use gnat_sim::circuit::Role;
use gnat_sim::{BrainPoints, Circuit, Point};
use std::sync::{Arc, Mutex};

use crate::font;

/// Spikes handed from the sim to the view. Bounded, because a view that stops
/// draining must not grow the sim's memory.
#[derive(Default)]
pub struct SpikeBus {
    events: Mutex<Vec<(usize, bool)>>,
}

const BUS_CAPACITY: usize = 256;

impl SpikeBus {
    pub fn push(&self, spikes: &[(usize, bool)]) {
        let mut events = self.events.lock().unwrap();
        events.extend_from_slice(spikes);
        if events.len() > BUS_CAPACITY {
            let excess = events.len() - BUS_CAPACITY;
            events.drain(..excess);
        }
    }

    pub fn drain(&self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.events.lock().unwrap())
    }
}

#[derive(Clone, Debug)]
pub struct Stim {
    pub neurons: Vec<usize>,
    pub strength: f32,
    pub duration_ms: i64,
}

/// Stimulation requests travelling the other way, from a click to the sim.
#[derive(Default)]
pub struct StimQueue {
    pending: Mutex<Vec<Stim>>,
}

impl StimQueue {
    pub fn push(&self, stim: Stim) {
        let mut pending = self.pending.lock().unwrap();
        pending.push(stim);
        // Bounded like the original's, so a click storm cannot grow it.
        if pending.len() > 8 {
            pending.remove(0);
        }
    }

    pub fn drain(&self) -> Vec<Stim> {
        std::mem::take(&mut self.pending.lock().unwrap())
    }
}

/// Everything the view needs that never changes, shared read-only.
pub struct BrainData {
    pub cloud: Vec<Point>,
    pub neurons: Vec<Neuron>,
}

pub struct Neuron {
    pub pos: [f32; 3],
    pub role: Role,
    pub cell_type: String,
}

impl BrainData {
    pub fn new(circuit: &Circuit, points: BrainPoints) -> Self {
        Self {
            cloud: points.points,
            neurons: circuit
                .neurons
                .iter()
                .map(|n| Neuron {
                    pos: n.pos,
                    role: n.role,
                    cell_type: n.cell_type.clone(),
                })
                .collect(),
        }
    }
}

/// Super-class colours for the cloud, from the original. Optic dominates by
/// count, so it is kept deliberately dim.
const CLASS_COLOURS: [[f32; 3]; 9] = [
    [0.16, 0.22, 0.34], // optic
    [0.45, 0.33, 0.16], // central
    [0.14, 0.36, 0.34], // sensory
    [0.10, 0.48, 0.62], // visual_projection
    [0.38, 0.22, 0.55], // visual_centrifugal
    [0.62, 0.28, 0.10], // descending
    [0.20, 0.45, 0.18], // ascending
    [0.55, 0.14, 0.14], // motor
    [0.50, 0.25, 0.40], // endocrine
];

fn role_colour(role: Role) -> [f32; 3] {
    match role {
        Role::Lc4 | Role::Lplc2 => [0.15, 0.85, 1.00],
        Role::Dna01 | Role::Dna02 => [1.00, 0.55, 0.10],
        Role::Mdn => [1.00, 0.20, 0.80],
        Role::Dnp09 => [0.25, 1.00, 0.35],
        Role::Dng11 => [0.75, 0.55, 1.00],
        Role::Escw => [1.00, 0.35, 0.25],
        Role::Gf => [1.00, 0.95, 0.40],
        Role::Other => [0.45, 0.45, 0.50],
    }
}

const BACKGROUND: Rgba = Rgba::opaque(8, 9, 16);
/// Vertical field of view, radians.
const FOV: f32 = 46.0 * std::f32::consts::PI / 180.0;
const CAM_Y: f32 = 0.6;
const CAM_Z: f32 = 29.0;
const PITCH: f32 = -0.15;
/// Radians per second, matching the original's six-second turn.
const SPIN: f32 = 0.35;
/// How far a click reaches for neighbours, in brain units.
const PICK_RADIUS: f32 = 2.2;
const LABEL_SECONDS: f32 = 3.0;

struct Flash {
    pos: [f32; 3],
    age: f32,
    life: f32,
    gf: bool,
}

pub struct View {
    data: Arc<BrainData>,
    bus: Arc<SpikeBus>,
    stims: Arc<StimQueue>,
    yaw: f32,
    last_ms: u64,
    flashes: Vec<Flash>,
    label: Option<(String, f32)>,
    /// Circuit neurons projected on the last frame: `(x, y, depth)`.
    projected: Vec<(f32, f32, f32)>,
    accum: Vec<f32>,
}

impl View {
    pub fn new(data: Arc<BrainData>, bus: Arc<SpikeBus>, stims: Arc<StimQueue>) -> Self {
        Self {
            data,
            bus,
            stims,
            yaw: 0.0,
            last_ms: 0,
            flashes: Vec::new(),
            label: None,
            projected: Vec::new(),
            accum: Vec::new(),
        }
    }

    /// Rotate a brain-space point into camera space and project it.
    ///
    /// Returns `(x, y, depth)` in pixels, or `None` when it is behind the
    /// camera.
    fn project(&self, p: [f32; 3], w: f32, h: f32) -> Option<(f32, f32, f32)> {
        let (sy, cy) = self.yaw.sin_cos();
        let (x1, z1) = (p[0] * cy + p[2] * sy, -p[0] * sy + p[2] * cy);
        let (sp, cp) = PITCH.sin_cos();
        let (y2, z2) = (p[1] * cp - z1 * sp, p[1] * sp + z1 * cp);

        let depth = CAM_Z - z2;
        if depth < 1.0 {
            return None;
        }
        let f = (h / 2.0) / (FOV / 2.0).tan();
        Some((
            w / 2.0 + f * x1 / depth,
            h / 2.0 - f * (y2 - CAM_Y) / depth,
            depth,
        ))
    }

    /// Add light at a point. `size` carries the canvas dimensions, which every
    /// caller already has to hand.
    fn add(
        &mut self,
        x: f32,
        y: f32,
        colour: [f32; 3],
        intensity: f32,
        radius: i32,
        size: (u32, u32),
    ) {
        let (w, h) = size;
        let (cx, cy) = (x.round() as i32, y.round() as i32);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let (px, py) = (cx + dx, cy + dy);
                if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                    continue;
                }
                let i = (py as usize * w as usize + px as usize) * 3;
                for (slot, c) in self.accum[i..i + 3].iter_mut().zip(colour) {
                    *slot += c * intensity;
                }
            }
        }
    }

    pub fn frame(&mut self, canvas: &mut Canvas, clicks: &[Click]) -> Flow {
        let (w, h) = (canvas.width, canvas.height);
        let dt = (canvas.time_ms.saturating_sub(self.last_ms) as f32 / 1000.0).clamp(0.0, 0.1);
        self.last_ms = canvas.time_ms;
        self.yaw += SPIN * dt;

        self.accum.clear();
        self.accum.resize(w as usize * h as usize * 3, 0.0);

        self.draw_cloud(w, h);
        self.draw_neurons(w, h);

        for (neuron, gf) in self.bus.drain() {
            self.add_flash(neuron, gf);
        }
        for click in clicks {
            self.handle_click(*click);
        }
        self.draw_flashes(dt, w, h);

        self.tone_map(canvas);
        self.draw_overlay(canvas, dt);
        Flow::Continue
    }

    fn draw_cloud(&mut self, w: u32, h: u32) {
        // Cloned out of the Arc-free path: the projection borrows `self`, and
        // the cloud is only read.
        let cloud = self.data.clone();
        for p in &cloud.cloud {
            let Some((x, y, depth)) = self.project(p.pos, w as f32, h as f32) else {
                continue;
            };
            let colour = CLASS_COLOURS
                .get(p.class as usize)
                .copied()
                .unwrap_or([0.3, 0.3, 0.3]);
            // Nearer somas are brighter, which is the only depth cue an
            // order-independent additive render gets.
            let intensity = (24.0 / depth).clamp(0.25, 1.4) * 0.9;
            self.add(x, y, colour, intensity, 0, (w, h));
        }
    }

    fn draw_neurons(&mut self, w: u32, h: u32) {
        let data = self.data.clone();
        self.projected.clear();
        self.projected.reserve(data.neurons.len());
        for n in &data.neurons {
            match self.project(n.pos, w as f32, h as f32) {
                Some((x, y, depth)) => {
                    self.projected.push((x, y, depth));
                    let intensity = (24.0 / depth).clamp(0.3, 1.6);
                    // The giant fibres get a permanent glow; they are two
                    // neurons out of 668 and the whole escape hangs off them.
                    let radius = if n.role == Role::Gf { 2 } else { 1 };
                    self.add(x, y, role_colour(n.role), intensity, radius, (w, h));
                }
                // Behind the camera: keep the index aligned with `neurons`.
                None => self.projected.push((f32::NAN, f32::NAN, f32::MAX)),
            }
        }
    }

    fn add_flash(&mut self, neuron: usize, gf: bool) {
        let Some(n) = self.data.neurons.get(neuron) else {
            return;
        };
        self.flashes.push(Flash {
            pos: n.pos,
            age: 0.0,
            life: if gf { 0.6 } else { 0.28 },
            gf,
        });
        // Same pool size as the original; older flashes simply drop.
        if self.flashes.len() > 48 {
            self.flashes.remove(0);
        }
    }

    fn draw_flashes(&mut self, dt: f32, w: u32, h: u32) {
        let mut live = std::mem::take(&mut self.flashes);
        for f in &mut live {
            f.age += dt;
        }
        live.retain(|f| f.age < f.life);
        for f in &live {
            let Some((x, y, depth)) = self.project(f.pos, w as f32, h as f32) else {
                continue;
            };
            let fade = 1.0 - f.age / f.life;
            let intensity = fade * (24.0 / depth).clamp(0.3, 1.6) * if f.gf { 3.0 } else { 1.4 };
            let radius = if f.gf { 5 } else { 2 };
            self.add(x, y, [0.75, 0.95, 1.0], intensity, radius, (w, h));
        }
        self.flashes = live;
    }

    /// Nearest circuit neuron to the click, then its neighbours in 3D.
    fn handle_click(&mut self, click: Click) {
        let mut best = None;
        let mut best_d2 = f32::MAX;
        for (i, &(x, y, _)) in self.projected.iter().enumerate() {
            if x.is_nan() {
                continue;
            }
            let d2 = (x - click.x).powi(2) + (y - click.y).powi(2);
            if d2 < best_d2 {
                best_d2 = d2;
                best = Some(i);
            }
        }
        let Some(anchor_idx) = best else { return };
        let anchor = self.data.neurons[anchor_idx].pos;

        let dist2 = |a: [f32; 3], b: [f32; 3]| {
            (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
        };
        let mut picked: Vec<usize> = (0..self.data.neurons.len())
            .filter(|&i| dist2(self.data.neurons[i].pos, anchor) < PICK_RADIUS * PICK_RADIUS)
            .collect();

        // A click in a sparse region should still do something, and one in a
        // dense region should not stimulate half the brain.
        if picked.len() < 4 || picked.len() > 60 {
            let mut all: Vec<usize> = (0..self.data.neurons.len()).collect();
            all.sort_by(|&a, &b| {
                dist2(self.data.neurons[a].pos, anchor)
                    .total_cmp(&dist2(self.data.neurons[b].pos, anchor))
            });
            let take = if picked.len() < 4 { 6 } else { 60 };
            picked = all.into_iter().take(take).collect();
        }

        self.label = Some((self.region_name(&picked), 0.0));
        for &i in picked.iter().take(16) {
            self.add_flash(i, false);
        }
        self.stims.push(Stim {
            neurons: picked,
            strength: 0.25,
            duration_ms: 400,
        });
    }

    /// What the clicked cluster mostly is.
    fn region_name(&self, picked: &[usize]) -> String {
        let mut counts: std::collections::HashMap<Role, usize> = std::collections::HashMap::new();
        for &i in picked {
            *counts.entry(self.data.neurons[i].role).or_default() += 1;
        }
        let Some((&major, _)) = counts.iter().max_by_key(|(_, n)| **n) else {
            return String::new();
        };

        let side = || {
            let left = picked
                .iter()
                .filter(|&&i| {
                    self.data.neurons[i].role == major && self.data.neurons[i].pos[0] < 0.0
                })
                .count();
            let total = picked
                .iter()
                .filter(|&&i| self.data.neurons[i].role == major)
                .count();
            let right = total - left;
            match left.cmp(&right) {
                std::cmp::Ordering::Equal => "",
                std::cmp::Ordering::Greater => " - LEFT",
                std::cmp::Ordering::Less => " - RIGHT",
            }
        };

        match major {
            Role::Lc4 | Role::Lplc2 => format!("LOOMING DETECTORS (LC4/LPLC2){}", side()),
            Role::Gf => "GIANT FIBRE (DNP01) - ESCAPE!".into(),
            Role::Dna01 | Role::Dna02 => format!("STEERING NEURONS (DNA01/02){}", side()),
            Role::Dnp09 => "WALKING COMMAND (DNP09)".into(),
            Role::Dng11 => "GROOMING COMMAND (DNG11)".into(),
            Role::Escw => "ESCAPE-WING DNS (DNP02/04/11)".into(),
            Role::Mdn => "MOONWALKER NEURONS (MDN)".into(),
            Role::Other => {
                let ty = picked
                    .iter()
                    .map(|&i| self.data.neurons[i].cell_type.as_str())
                    .find(|t| !t.is_empty() && *t != "?")
                    .unwrap_or("CENTRAL");
                format!("{} NEURONS", ty.to_ascii_uppercase())
            }
        }
    }

    /// Accumulated light into pixels.
    fn tone_map(&self, canvas: &mut Canvas) {
        for y in 0..canvas.height {
            for x in 0..canvas.width {
                let i = (y as usize * canvas.width as usize + x as usize) * 3;
                // Filmic-ish: saturates smoothly instead of clipping, so a
                // dense cluster reads as bright rather than as a white blob.
                let map = |v: f32| {
                    let t = 1.0 - (-v * 1.6).exp();
                    (t * 255.0).clamp(0.0, 255.0) as u8
                };
                let (r, g, b) = (
                    map(self.accum[i]),
                    map(self.accum[i + 1]),
                    map(self.accum[i + 2]),
                );
                canvas.put(
                    x as i32,
                    y as i32,
                    Rgba::opaque(
                        r.max(BACKGROUND.r),
                        g.max(BACKGROUND.g),
                        b.max(BACKGROUND.b),
                    ),
                );
            }
        }
    }

    fn draw_overlay(&mut self, canvas: &mut Canvas, dt: f32) {
        font::draw(
            canvas,
            8,
            8,
            "FLY BRAIN - FLYWIRE V783",
            2,
            Rgba::opaque(120, 140, 170),
        );
        font::draw(
            canvas,
            8,
            8 + font::height(2) + 6,
            "CLICK = STIMULATE",
            1,
            Rgba::opaque(80, 95, 120),
        );

        if let Some((text, age)) = &mut self.label {
            *age += dt;
            if *age > LABEL_SECONDS {
                self.label = None;
            } else {
                let text = text.clone();
                let age = *age;
                let scale = 2;
                let y = canvas.height as i32 - font::height(scale) - 10;
                // Centred, and faded out over the last second rather than
                // vanishing mid-read.
                let x = (canvas.width as i32 - font::width(&text, scale)) / 2;
                let alpha = (LABEL_SECONDS - age).clamp(0.0, 1.0);
                let c = Rgba::new(230, 240, 255, (alpha * 255.0) as u8);
                font::draw(canvas, x.max(4), y, &text, scale, c);
            }
        }
    }
}

/// Render frames without a compositor, for `--brainshot`.
pub fn render_offscreen(
    view: &mut View,
    width: u32,
    height: u32,
    frames: u32,
    mut before: impl FnMut(u32),
) -> Vec<u8> {
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for i in 0..frames.max(1) {
        before(i);
        let mut canvas = Canvas {
            width,
            height,
            pixels: &mut pixels,
            time_ms: (i as u64) * 1000 / 60,
        };
        view.frame(&mut canvas, &[]);
    }
    pixels
}

/// Open the brain window and run it until it is closed.
pub fn run(data: Arc<BrainData>, bus: Arc<SpikeBus>, stims: Arc<StimQueue>) -> Result<()> {
    let mut window = Window::new(WindowConfig {
        title: "Fly Brain - FlyWire v783 (click = stimulate)".into(),
        width: 560,
        height: 460,
        ..WindowConfig::default()
    })?;
    let mut view = View::new(data, bus, stims);
    window.run(move |canvas, clicks| view.frame(canvas, clicks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnat_sim::circuit::Side;

    fn neuron(pos: [f32; 3], role: Role) -> Neuron {
        Neuron {
            pos,
            role,
            cell_type: "central".into(),
        }
    }

    fn view(neurons: Vec<Neuron>) -> View {
        let data = Arc::new(BrainData {
            cloud: Vec::new(),
            neurons,
        });
        View::new(data, Arc::default(), Arc::default())
    }

    #[test]
    fn the_spike_bus_is_bounded() {
        let bus = SpikeBus::default();
        for i in 0..BUS_CAPACITY * 3 {
            bus.push(&[(i, false)]);
        }
        let drained = bus.drain();
        assert_eq!(drained.len(), BUS_CAPACITY);
        // The newest spikes survive; a view that stalls should resume with
        // current activity, not with a backlog.
        assert_eq!(drained.last().unwrap().0, BUS_CAPACITY * 3 - 1);
        assert!(bus.drain().is_empty(), "draining should empty the bus");
    }

    #[test]
    fn the_stim_queue_is_bounded() {
        let q = StimQueue::default();
        for i in 0..20 {
            q.push(Stim {
                neurons: vec![i],
                strength: 0.25,
                duration_ms: 400,
            });
        }
        let drained = q.drain();
        assert_eq!(drained.len(), 8);
        assert_eq!(drained.last().unwrap().neurons, vec![19]);
    }

    #[test]
    fn the_origin_projects_near_the_middle_of_the_view() {
        let v = view(Vec::new());
        let (x, y, depth) = v.project([0.0, 0.0, 0.0], 600.0, 400.0).unwrap();
        assert!((x - 300.0).abs() < 1.0, "x was {x}");
        // The camera sits slightly above the origin and looks down a little,
        // so dead centre is not exactly the middle row.
        assert!((y - 200.0).abs() < 30.0, "y was {y}");
        assert!((depth - CAM_Z).abs() < 1.0, "depth was {depth}");
    }

    #[test]
    fn points_behind_the_camera_are_dropped() {
        let v = view(Vec::new());
        assert!(v.project([0.0, 0.0, 100.0], 600.0, 400.0).is_none());
    }

    #[test]
    fn nearer_points_project_further_from_the_centre() {
        let v = view(Vec::new());
        let near = v.project([5.0, 0.0, 10.0], 600.0, 400.0).unwrap();
        let far = v.project([5.0, 0.0, -10.0], 600.0, 400.0).unwrap();
        assert!(
            (near.0 - 300.0).abs() > (far.0 - 300.0).abs(),
            "perspective is inverted: near {} far {}",
            near.0,
            far.0
        );
    }

    #[test]
    fn a_click_stimulates_the_cluster_it_landed_on() {
        // Two well-separated clusters; a click on one must not reach the other.
        let mut neurons = Vec::new();
        for i in 0..6 {
            neurons.push(neuron([-8.0 + i as f32 * 0.2, 0.0, 0.0], Role::Lc4));
        }
        for i in 0..6 {
            neurons.push(neuron([8.0 + i as f32 * 0.2, 0.0, 0.0], Role::Dnp09));
        }
        let mut v = view(neurons);

        let mut px = vec![0u8; 600 * 400 * 4];
        let mut canvas = Canvas {
            width: 600,
            height: 400,
            pixels: &mut px,
            time_ms: 0,
        };
        // One frame to populate the projection, then click where the first
        // cluster landed.
        v.frame(&mut canvas, &[]);
        let target = v.projected[0];
        v.frame(
            &mut canvas,
            &[Click {
                x: target.0,
                y: target.1,
            }],
        );

        let stims = v.stims.drain();
        assert_eq!(stims.len(), 1, "exactly one stimulation per click");
        assert!(
            stims[0].neurons.iter().all(|&i| i < 6),
            "the click reached the far cluster: {:?}",
            stims[0].neurons
        );
        assert!(v.label.is_some(), "a click should name what it hit");
    }

    #[test]
    fn clusters_are_named_by_their_majority_role() {
        let v = view(vec![
            neuron([-1.0, 0.0, 0.0], Role::Gf),
            neuron([1.0, 0.0, 0.0], Role::Gf),
            neuron([0.0, 0.0, 0.0], Role::Other),
        ]);
        assert_eq!(v.region_name(&[0, 1, 2]), "GIANT FIBRE (DNP01) - ESCAPE!");
        assert_eq!(v.region_name(&[2]), "CENTRAL NEURONS");
    }

    #[test]
    fn a_one_sided_cluster_says_which_side() {
        let v = view(vec![
            neuron([-5.0, 0.0, 0.0], Role::Lc4),
            neuron([-4.0, 0.0, 0.0], Role::Lc4),
        ]);
        assert!(
            v.region_name(&[0, 1]).ends_with(" - LEFT"),
            "{}",
            v.region_name(&[0, 1])
        );

        let balanced = view(vec![
            neuron([-5.0, 0.0, 0.0], Role::Lc4),
            neuron([5.0, 0.0, 0.0], Role::Lc4),
        ]);
        assert!(
            !balanced.region_name(&[0, 1]).contains(" - "),
            "a balanced cluster should not claim a side"
        );
    }

    #[test]
    fn the_flash_pool_is_bounded() {
        let mut v = view(
            (0..200)
                .map(|i| neuron([i as f32, 0.0, 0.0], Role::Other))
                .collect(),
        );
        for i in 0..200 {
            v.add_flash(i, false);
        }
        assert_eq!(v.flashes.len(), 48);
    }

    #[test]
    fn a_flash_for_a_neuron_that_does_not_exist_is_ignored() {
        let mut v = view(vec![neuron([0.0, 0.0, 0.0], Role::Gf)]);
        v.add_flash(9999, true);
        assert!(v.flashes.is_empty());
    }

    #[test]
    fn every_role_and_side_has_a_colour() {
        for role in [
            Role::Lc4,
            Role::Lplc2,
            Role::Gf,
            Role::Dna01,
            Role::Dna02,
            Role::Mdn,
            Role::Dnp09,
            Role::Dng11,
            Role::Escw,
            Role::Other,
        ] {
            let c = role_colour(role);
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{role:?}");
        }
        // Side is part of the circuit's shape; make sure the import is real.
        assert_ne!(Side::Left, Side::Right);
    }
}
