//! Leaky integrate-and-fire simulation of the escape/steering circuit.
//!
//! A direct port of the original `Sim.swift`. Every constant below is the
//! upstream value; where the port diverges it is called out in a comment.
//!
//! The circuit is LC4/LPLC2 looming detectors driving the DNp01 giant fibre,
//! DNa01/DNa02 steering, MDN backward walking, DNp09 forward walking, DNg11
//! grooming, and the DNp02/04/11 escape-manoeuvre wing DNs, wired with real
//! signed FlyWire synapse counts.

use crate::circuit::{Circuit, Role, Side};
use crate::rng::Rng;

/// Membrane decay per millisecond: `exp(-1/20)`, a 20 ms time constant on a
/// 1 ms step.
const DECAY: f32 = 0.9512;
const THRESHOLD: f32 = 1.0;
/// Absolute refractory period, in milliseconds.
const REFRACTORY_MS: f32 = 2.0;
/// Turns a raw signed synapse count into a voltage step.
const WEIGHT_SCALE: f32 = 0.0008;
/// Per-neuron, per-millisecond probability of a spontaneous depolarisation.
const P_NOISE: f32 = 0.0022;
const NOISE_KICK: f32 = 0.42;
const LOOM_GAIN: f32 = 0.30;
/// Smoothing for the exponential moving average on group firing rates.
const RATE_ALPHA: f32 = 1.0 / 120.0;
/// Inhibition floor. Without it, a strongly inhibited neuron takes seconds to
/// climb back to threshold and the circuit latches silent.
const V_FLOOR: f32 = -2.0;

/// GABA and glutamate synapses arrive a few milliseconds late; the LC->GF
/// electrical coupling is instantaneous. That latency window is the whole
/// reason the giant fibre can fire before feedforward inhibition lands, and so
/// the reason the fly escapes at all.
const INH_DELAY_MS: usize = 4;
const INH_SLOTS: usize = INH_DELAY_MS + 1;

/// LC4/LPLC2 and the wind (Johnston's organ) pathway reach the giant fibre
/// through gap junctions, which chemical synapse counts under-represent.
const GAP_JUNCTION_BOOST: f32 = 6.0;

/// Occasional arousal bursts multiply the noise rate for 400 ms.
const BURST_MS: i64 = 400;
const BURST_NOISE_FACTOR: f32 = 6.0;
const FIRST_BURST_MS: i64 = 12_000;
const BURST_GAP_MS: (i64, i64) = (15_000, 40_000);

/// Optogenetic-style stimulation, as delivered by a click in the brain view.
#[derive(Clone, Debug)]
struct Stim {
    idx: Vec<usize>,
    strength: f32,
    until_ms: i64,
}

/// Index sets for each population, resolved once at construction.
#[derive(Clone, Debug, Default)]
pub struct Groups {
    pub loom_left: Vec<usize>,
    pub loom_right: Vec<usize>,
    pub gf: Vec<usize>,
    /// DNa01 + DNa02, left.
    pub dna_l: Vec<usize>,
    /// DNa01 + DNa02, right.
    pub dna_r: Vec<usize>,
    pub mdn: Vec<usize>,
    /// DNp09, forward walking.
    pub fwd: Vec<usize>,
    /// DNg11, grooming.
    pub groom: Vec<usize>,
    /// DNp02/04/11, escape manoeuvre.
    pub escw: Vec<usize>,
    /// Ascending partners, carrying leg proprioception.
    pub ascend: Vec<usize>,
    /// Sensory partners, the air-puff pathway.
    pub sens: Vec<usize>,
}

/// Group firing rates, in Hz per neuron, exponentially smoothed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rates {
    pub loom: f32,
    pub dna_l: f32,
    pub dna_r: f32,
    pub mdn: f32,
    pub fwd: f32,
    pub groom: f32,
    pub escw: f32,
    /// Whole-population rate.
    pub pop: f32,
}

/// What the coordinator feeds in each frame. All in 0..1 unless noted.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    /// Looming drive on the left eye.
    pub loom_l: f32,
    pub loom_r: f32,
    /// Walking intensity, into the ascending proprioceptive neurons.
    pub gait_drive: f32,
    /// Body gait phase, 0..1.
    pub gait_phase: f32,
    /// Fast cursor motion near the fly, into the sensory pathway.
    pub air_puff: f32,
    /// Circadian and sleep neuromodulation of baseline drive and noise.
    pub activity_scale: f32,
    /// Sleep gates sensory input by raising the arousal threshold.
    pub sensory_gate: f32,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            loom_l: 0.0,
            loom_r: 0.0,
            gait_drive: 0.0,
            gait_phase: 0.0,
            air_puff: 0.0,
            activity_scale: 1.0,
            sensory_gate: 1.0,
        }
    }
}

pub struct Lif {
    pub circuit: Circuit,
    pub inputs: Inputs,

    n: usize,
    roles: Vec<Role>,

    v: Vec<f32>,
    refr: Vec<f32>,
    /// Per-neuron constant drive. Heterogeneous, so interneurons crackle at a
    /// few Hz while command neurons stay quiet unless driven.
    baseline: Vec<f32>,

    // CSR adjacency, weights pre-scaled.
    row_start: Vec<usize>,
    col_idx: Vec<u32>,
    w: Vec<f32>,

    groups: Groups,
    /// True for a steering DN on the left, for O(1) rate bucketing.
    dna_is_left: Vec<bool>,
    /// Per-ascending-neuron gait phase offset.
    ascend_phase: Vec<f32>,

    rates: Rates,
    gf_latch: bool,
    sim_ms: i64,
    total_spikes: u64,

    inh_queue: Vec<Vec<f32>>,
    q_head: usize,

    burst_until: i64,
    burst_next: i64,

    stims: Vec<Stim>,
    /// Spikes sampled for the brain view since the last drain.
    spike_log: Vec<(usize, bool)>,
    log_spikes: bool,

    rng: Rng,
}

impl Lif {
    pub fn new(circuit: Circuit, seed: u64) -> Self {
        let n = circuit.len();
        let mut rng = Rng::new(seed);
        let roles: Vec<Role> = circuit.neurons.iter().map(|nr| nr.role).collect();

        let mut groups = Groups::default();
        for (i, nr) in circuit.neurons.iter().enumerate() {
            match nr.role {
                Role::Lc4 | Role::Lplc2 => {
                    // `side` is left or right for every optic-lobe neuron; the
                    // upstream code buckets anything not-left as right.
                    if nr.side == Side::Left {
                        groups.loom_left.push(i)
                    } else {
                        groups.loom_right.push(i)
                    }
                }
                Role::Gf => groups.gf.push(i),
                Role::Dna01 | Role::Dna02 => {
                    if nr.side == Side::Left {
                        groups.dna_l.push(i)
                    } else {
                        groups.dna_r.push(i)
                    }
                }
                Role::Mdn => groups.mdn.push(i),
                Role::Dnp09 => groups.fwd.push(i),
                Role::Dng11 => groups.groom.push(i),
                Role::Escw => groups.escw.push(i),
                Role::Other => {
                    // Partners keep their super class as `type`.
                    match nr.cell_type.as_str() {
                        "ascending" => groups.ascend.push(i),
                        "sensory" => groups.sens.push(i),
                        _ => {}
                    }
                }
            }
        }

        let mut dna_is_left = vec![false; n];
        for &i in &groups.dna_l {
            dna_is_left[i] = true;
        }

        let ascend_phase = groups
            .ascend
            .iter()
            .map(|_| rng.range(0.0, 2.0 * std::f32::consts::PI))
            .collect();

        let baseline = circuit
            .neurons
            .iter()
            .map(|nr| match nr.role {
                Role::Other => rng.range(0.010, 0.070),
                Role::Lc4 | Role::Lplc2 => 0.004,
                // Command DNs get deterministic, side-symmetric baselines:
                // their asymmetries and bursts must come from network dynamics
                // rather than from luck.
                Role::Dna01 | Role::Dna02 | Role::Mdn | Role::Dng11 | Role::Escw => 0.036,
                Role::Dnp09 => 0.038,
                // The giant fibre stays quiet unless synaptically driven.
                Role::Gf => 0.002,
            })
            .collect();

        // Build CSR.
        let mut counts = vec![0usize; n];
        for &(pre, _, _) in &circuit.edges {
            counts[pre as usize] += 1;
        }
        let mut row_start = vec![0usize; n + 1];
        for i in 0..n {
            row_start[i + 1] = row_start[i] + counts[i];
        }
        let mut col_idx = vec![0u32; circuit.edges.len()];
        let mut w = vec![0.0f32; circuit.edges.len()];
        let mut fill = row_start.clone();
        for &(pre, post, syn) in &circuit.edges {
            let (pre, post) = (pre as usize, post as usize);
            let mut weight = syn * WEIGHT_SCALE;
            let electrical = roles[pre].is_looming()
                || (roles[pre] == Role::Other && circuit.neurons[pre].cell_type == "sensory");
            if electrical && roles[post] == Role::Gf {
                weight *= GAP_JUNCTION_BOOST;
            }
            col_idx[fill[pre]] = post as u32;
            w[fill[pre]] = weight;
            fill[pre] += 1;
        }

        Self {
            circuit,
            inputs: Inputs::default(),
            n,
            roles,
            v: vec![0.0; n],
            refr: vec![0.0; n],
            baseline,
            row_start,
            col_idx,
            w,
            groups,
            dna_is_left,
            ascend_phase,
            rates: Rates::default(),
            gf_latch: false,
            sim_ms: 0,
            total_spikes: 0,
            inh_queue: vec![vec![0.0; n]; INH_SLOTS],
            q_head: 0,
            burst_until: 0,
            burst_next: FIRST_BURST_MS,
            stims: Vec::new(),
            spike_log: Vec::new(),
            log_spikes: false,
            rng,
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn groups(&self) -> &Groups {
        &self.groups
    }

    pub fn rates(&self) -> Rates {
        self.rates
    }

    pub fn sim_ms(&self) -> i64 {
        self.sim_ms
    }

    pub fn total_spikes(&self) -> u64 {
        self.total_spikes
    }

    pub fn potentials(&self) -> &[f32] {
        &self.v
    }

    /// Whether the giant fibre has fired since this was last called. Reading it
    /// clears the latch, so exactly one caller may consume it.
    pub fn consume_gf(&mut self) -> bool {
        std::mem::replace(&mut self.gf_latch, false)
    }

    /// Start sampling spikes for the brain view. Off by default, because the
    /// sampling is pure overhead when nothing is drawing.
    pub fn set_spike_logging(&mut self, on: bool) {
        self.log_spikes = on;
        if !on {
            self.spike_log.clear();
        }
    }

    /// Take the sampled spikes as `(neuron, is_giant_fibre)`.
    pub fn drain_spikes(&mut self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.spike_log)
    }

    /// Inject current into a population for a fixed duration — what a click in
    /// the brain view does.
    pub fn stimulate(&mut self, indices: &[usize], strength: f32, duration_ms: i64) {
        if indices.is_empty() {
            return;
        }
        self.stims.push(Stim {
            idx: indices.to_vec(),
            strength,
            until_ms: self.sim_ms + duration_ms,
        });
        // Bound the queue the way the original does, so a click storm cannot
        // grow it without limit.
        if self.stims.len() > 8 {
            self.stims.remove(0);
        }
    }

    /// Advance the simulation by `ms` milliseconds.
    pub fn step(&mut self, ms: i64) {
        if ms <= 0 {
            return;
        }
        for _ in 0..ms {
            self.step_one_ms();
        }
    }

    fn step_one_ms(&mut self) {
        self.sim_ms += 1;
        let now = self.sim_ms;
        self.stims.retain(|s| now < s.until_ms);

        if now >= self.burst_next {
            self.burst_until = now + BURST_MS;
            self.burst_next = now + self.rng.range_i(BURST_GAP_MS.0, BURST_GAP_MS.1);
        }
        let inp = self.inputs;
        let p = if now < self.burst_until {
            P_NOISE * BURST_NOISE_FACTOR
        } else {
            P_NOISE
        } * inp.activity_scale;

        // Leak, baseline drive, and spontaneous noise.
        for i in 0..self.n {
            if self.refr[i] > 0.0 {
                self.refr[i] -= 1.0;
                self.v[i] *= DECAY;
                continue;
            }
            let mut vi = self.v[i] * DECAY + self.baseline[i] * inp.activity_scale;
            if self.rng.f32() < p {
                vi += NOISE_KICK;
            }
            self.v[i] = vi;
        }

        // Sensory drive.
        if inp.loom_l > 0.001 {
            let d = inp.loom_l * LOOM_GAIN * inp.sensory_gate;
            for &i in &self.groups.loom_left {
                self.v[i] += d;
            }
        }
        if inp.loom_r > 0.001 {
            let d = inp.loom_r * LOOM_GAIN * inp.sensory_gate;
            for &i in &self.groups.loom_right {
                self.v[i] += d;
            }
        }
        // Body to brain: gait rhythm into the ascending proprioceptive neurons.
        if inp.gait_drive > 0.001 {
            let ph = inp.gait_phase * 2.0 * std::f32::consts::PI;
            for (k, &i) in self.groups.ascend.iter().enumerate() {
                self.v[i] +=
                    inp.gait_drive * 0.09 * (0.5 + 0.5 * (ph + self.ascend_phase[k]).sin());
            }
        }
        if inp.air_puff > 0.001 {
            let d = inp.air_puff * 0.12 * inp.sensory_gate;
            for &i in &self.groups.sens {
                self.v[i] += d;
            }
        }
        for s in &self.stims {
            for &i in &s.idx {
                self.v[i] += s.strength;
            }
        }

        // Deliver the inhibition scheduled for this millisecond.
        {
            let slot = &mut self.inh_queue[self.q_head];
            for (vi, q) in self.v.iter_mut().zip(slot.iter_mut()) {
                if *q != 0.0 {
                    *vi = (*vi + *q).max(V_FLOOR);
                    *q = 0.0;
                }
            }
        }

        // Threshold.
        let mut spiked: Vec<usize> = Vec::new();
        for i in 0..self.n {
            if self.refr[i] <= 0.0 && self.v[i] >= THRESHOLD {
                self.v[i] = 0.0;
                self.refr[i] = REFRACTORY_MS;
                spiked.push(i);
            }
        }
        self.total_spikes += spiked.len() as u64;

        // Propagate: excitation lands immediately, inhibition is queued.
        let inh_slot = (self.q_head + INH_DELAY_MS) % INH_SLOTS;
        for &i in &spiked {
            for k in self.row_start[i]..self.row_start[i + 1] {
                let j = self.col_idx[k] as usize;
                let w = self.w[k];
                if w >= 0.0 {
                    self.v[j] = (self.v[j] + w).max(V_FLOOR);
                } else {
                    self.inh_queue[inh_slot][j] += w;
                }
            }
        }
        self.q_head = (self.q_head + 1) % INH_SLOTS;

        self.update_rates(&spiked);

        if self.log_spikes && !spiked.is_empty() {
            // Sample under heavy activity: the brain view only needs a flavour
            // of the traffic, not every spike.
            let stride = (spiked.len() / 12).max(1);
            for &i in spiked.iter().step_by(stride) {
                self.spike_log.push((i, self.roles[i] == Role::Gf));
            }
        }
    }

    fn update_rates(&mut self, spiked: &[usize]) {
        let (mut c_loom, mut c_dl, mut c_dr) = (0u32, 0u32, 0u32);
        let (mut c_m, mut c_f, mut c_g, mut c_w) = (0u32, 0u32, 0u32, 0u32);
        for &i in spiked {
            match self.roles[i] {
                Role::Lc4 | Role::Lplc2 => c_loom += 1,
                Role::Dna01 | Role::Dna02 => {
                    if self.dna_is_left[i] {
                        c_dl += 1
                    } else {
                        c_dr += 1
                    }
                }
                Role::Mdn => c_m += 1,
                Role::Dnp09 => c_f += 1,
                Role::Dng11 => c_g += 1,
                Role::Escw => c_w += 1,
                Role::Gf => self.gf_latch = true,
                Role::Other => {}
            }
        }

        // Spikes in one millisecond scale to Hz by a factor of 1000.
        let hz = |count: u32, pop: usize| count as f32 * 1000.0 / pop.max(1) as f32;
        let ema = |cur: &mut f32, target: f32| *cur += (target - *cur) * RATE_ALPHA;

        let n_loom = self.groups.loom_left.len() + self.groups.loom_right.len();
        ema(&mut self.rates.loom, hz(c_loom, n_loom));
        ema(&mut self.rates.dna_l, hz(c_dl, self.groups.dna_l.len()));
        ema(&mut self.rates.dna_r, hz(c_dr, self.groups.dna_r.len()));
        ema(&mut self.rates.mdn, hz(c_m, self.groups.mdn.len()));
        ema(&mut self.rates.fwd, hz(c_f, self.groups.fwd.len()));
        ema(&mut self.rates.groom, hz(c_g, self.groups.groom.len()));
        ema(&mut self.rates.escw, hz(c_w, self.groups.escw.len()));
        ema(&mut self.rates.pop, hz(spiked.len() as u32, self.n));
    }
}
