//! The headless circuit test, ported from the original's `--simtest`.
//!
//! Six phases that between them check the circuit is alive but not seizing:
//! silent giant fibre at rest, a giant-fibre volley on an abrupt loom,
//! locomotor drive that fluctuates rather than latching, a siesta that slows
//! the fly without paralysing it, and click stimulation that lands.

use crate::circuit::Circuit;
use crate::lif::Lif;

#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub neurons: usize,
    pub loom_l: usize,
    pub loom_r: usize,
    pub gf: usize,
    pub dna_l: usize,
    pub dna_r: usize,
    pub mdn: usize,
    pub fwd: usize,
    pub groom: usize,
    pub escw: usize,
    pub ascend: usize,
    pub sens: usize,

    /// Phase 1 — 4 s of spontaneous activity.
    pub spont_pop_hz: f32,
    pub spont_loom_hz: f32,
    pub spont_dna_l_hz: f32,
    pub spont_dna_r_hz: f32,
    pub spont_mdn_hz: f32,
    /// Must be zero: a giant fibre that fires at rest is a fly that never
    /// stops taking off.
    pub gf_spontaneous: u32,

    /// Phase 2 — abrupt loom, as produced by a cursor lunge.
    pub loom_rate_hz: f32,
    pub gf_on_loom: u32,
    /// Milliseconds from stimulus onset to the first giant-fibre spike.
    pub gf_latency_ms: i32,

    /// Phase 3 — 20 s with walking proprioception.
    pub walk_on_pct: f32,
    pub groom_on_pct: f32,
    pub fwd_min_hz: f32,
    pub fwd_max_hz: f32,
    pub pop_hz: f32,

    /// Phase 3b — midday siesta.
    pub siesta_walk_on_pct: f32,

    /// Phase 4 — 1 s air puff through the wind startle pathway.
    pub gf_on_puff: u32,

    /// Phase 5 — left-eye-only loom, the steering probe.
    pub dna_diff_before: f32,
    pub dna_diff_after: f32,
    pub steer_loom_hz: f32,

    /// Phase 6 — click stimulation, as the brain window delivers it.
    pub gf_stim_fired: bool,
    pub groom_stim_hz: f32,
}

impl Report {
    /// The upstream pass condition, unchanged.
    pub fn passed(&self) -> bool {
        self.gf_spontaneous == 0
            && self.gf_on_loom > 0
            && self.walk_on_pct > 0.0
            && self.gf_stim_fired
            && self.siesta_walk_on_pct > 3.0
    }
}

/// The compressed midday siesta scale: `1 - (1 - 0.55) * 0.35`.
const SIESTA_SCALE: f32 = 1.0 - (1.0 - 0.55) * 0.35;

pub fn run(circuit: Circuit, seed: u64) -> Report {
    let mut sim = Lif::new(circuit, seed);
    let g = sim.groups().clone();

    // --- Phase 1: 4 s spontaneous activity ---------------------------------
    let mut gf_spontaneous = 0;
    for _ in 0..40 {
        sim.step(100);
        if sim.consume_gf() {
            gf_spontaneous += 1;
        }
    }
    let spont_pop_hz = sim.total_spikes() as f32 / 4.0 / sim.len() as f32;
    let r1 = sim.rates();

    // --- Phase 2: abrupt loom (a step, not a ramp) -------------------------
    let mut gf_latency_ms = -1;
    let mut gf_on_loom = 0;
    for ms in 0..400 {
        sim.inputs.loom_l = 1.0;
        sim.inputs.loom_r = 0.5;
        sim.step(1);
        if sim.consume_gf() {
            gf_on_loom += 1;
            if gf_latency_ms < 0 {
                gf_latency_ms = ms;
            }
        }
    }
    sim.inputs.loom_l = 0.0;
    sim.inputs.loom_r = 0.0;
    let loom_rate_hz = sim.rates().loom;

    // --- Phase 3: 20 s with walking proprioception -------------------------
    let (mut walk_on, mut groom_on, mut samples) = (0u32, 0u32, 0u32);
    let mut fwd_min = f32::MAX;
    let mut fwd_max = 0.0f32;
    for ms in 0..20_000 {
        sim.inputs.gait_drive = 0.5;
        sim.inputs.gait_phase = (ms % 125) as f32 / 125.0; // 8 Hz gait
        sim.step(1);
        if ms % 10 == 0 {
            samples += 1;
            let r = sim.rates();
            if r.fwd / 10.0 > 0.22 {
                walk_on += 1;
            }
            if r.groom / 8.0 > 0.5 {
                groom_on += 1;
            }
            fwd_min = fwd_min.min(r.fwd);
            fwd_max = fwd_max.max(r.fwd);
        }
    }
    let r3 = sim.rates();
    let pct = |on: u32, total: u32| 100.0 * on as f32 / total.max(1) as f32;

    // --- Phase 3b: the siesta must slow the fly, not paralyse it -----------
    sim.inputs.activity_scale = SIESTA_SCALE;
    let (mut siesta_walk_on, mut siesta_samples) = (0u32, 0u32);
    for ms in 0..15_000 {
        sim.step(1);
        if ms % 10 == 0 {
            siesta_samples += 1;
            if sim.rates().fwd / 10.0 > 0.22 {
                siesta_walk_on += 1;
            }
        }
    }
    sim.inputs.activity_scale = 1.0;

    // --- Phase 4: air puff, the wind startle pathway -----------------------
    let mut gf_on_puff = 0;
    for _ in 0..1000 {
        sim.inputs.air_puff = 1.0;
        sim.step(1);
        if sim.consume_gf() {
            gf_on_puff += 1;
        }
    }
    sim.inputs.air_puff = 0.0;

    // --- Phase 5: gentle left-eye-only loom, the steering probe ------------
    for _ in 0..500 {
        sim.step(1);
        let _ = sim.consume_gf();
    }
    let before = sim.rates();
    let dna_diff_before = before.dna_l - before.dna_r;
    for _ in 0..1000 {
        sim.inputs.loom_l = 0.30;
        sim.inputs.loom_r = 0.0;
        sim.step(1);
        let _ = sim.consume_gf();
    }
    let after = sim.rates();
    sim.inputs.loom_l = 0.0;

    // --- Phase 6: click stimulation ----------------------------------------
    sim.stimulate(&g.gf, 0.5, 40);
    sim.step(60);
    let gf_stim_fired = sim.consume_gf();
    sim.stimulate(&g.groom, 0.25, 400);
    sim.step(400);
    let groom_stim_hz = sim.rates().groom;
    let _ = sim.consume_gf();

    Report {
        neurons: sim.len(),
        loom_l: g.loom_left.len(),
        loom_r: g.loom_right.len(),
        gf: g.gf.len(),
        dna_l: g.dna_l.len(),
        dna_r: g.dna_r.len(),
        mdn: g.mdn.len(),
        fwd: g.fwd.len(),
        groom: g.groom.len(),
        escw: g.escw.len(),
        ascend: g.ascend.len(),
        sens: g.sens.len(),

        spont_pop_hz,
        spont_loom_hz: r1.loom,
        spont_dna_l_hz: r1.dna_l,
        spont_dna_r_hz: r1.dna_r,
        spont_mdn_hz: r1.mdn,
        gf_spontaneous,

        loom_rate_hz,
        gf_on_loom,
        gf_latency_ms,

        walk_on_pct: pct(walk_on, samples),
        groom_on_pct: pct(groom_on, samples),
        fwd_min_hz: fwd_min,
        fwd_max_hz: fwd_max,
        pop_hz: r3.pop,

        siesta_walk_on_pct: pct(siesta_walk_on, siesta_samples),

        gf_on_puff,

        dna_diff_before,
        dna_diff_after: after.dna_l - after.dna_r,
        steer_loom_hz: after.loom,

        gf_stim_fired,
        groom_stim_hz,
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "circuit: {} neurons | loom L/R: {}/{} | GF: {} | DNa L/R: {}/{} | MDN: {} \
             | DNp09: {} | DNg11: {} | escW: {} | ascend: {} | sens: {}",
            self.neurons,
            self.loom_l,
            self.loom_r,
            self.gf,
            self.dna_l,
            self.dna_r,
            self.mdn,
            self.fwd,
            self.groom,
            self.escw,
            self.ascend,
            self.sens
        )?;
        writeln!(
            f,
            "spontaneous 4s: pop {:.2} Hz/neuron, LC {:.1} Hz, DNa02 L/R {:.1}/{:.1} Hz, \
             MDN {:.1} Hz, GF spikes: {}",
            self.spont_pop_hz,
            self.spont_loom_hz,
            self.spont_dna_l_hz,
            self.spont_dna_r_hz,
            self.spont_mdn_hz,
            self.gf_spontaneous
        )?;
        writeln!(
            f,
            "abrupt loom 0.4s: LC rate {:.1} Hz, GF spikes {}, first at {} ms",
            self.loom_rate_hz, self.gf_on_loom, self.gf_latency_ms
        )?;
        writeln!(
            f,
            "behavior 20s: walk-drive on {:.0}%, groom-drive on {:.0}%, DNp09 {:.1}-{:.1} Hz, pop {:.1} Hz",
            self.walk_on_pct, self.groom_on_pct, self.fwd_min_hz, self.fwd_max_hz, self.pop_hz
        )?;
        writeln!(
            f,
            "siesta 15s (scale {SIESTA_SCALE:.2}): walk-drive on {:.0}%",
            self.siesta_walk_on_pct
        )?;
        writeln!(f, "air puff 1s: GF spikes {}", self.gf_on_puff)?;
        writeln!(
            f,
            "left-eye loom: DNa L-R rate diff {:+.1} -> {:+.1} Hz, LC {:.1} Hz",
            self.dna_diff_before, self.dna_diff_after, self.steer_loom_hz
        )?;
        writeln!(
            f,
            "click probes: GF cluster -> spike {}, DNg11 cluster -> groom rate {:.0} Hz",
            if self.gf_stim_fired { "yes" } else { "NO" },
            self.groom_stim_hz
        )?;
        write!(
            f,
            "{}",
            if self.passed() {
                "PASS: GF silent at rest, fires on loom; locomotor drive fluctuates; stim works; siesta alive"
            } else {
                "FAIL: tune weights/noise"
            }
        )
    }
}
