//! The leaky integrate-and-fire loop.
//!
//! Runs on a fixed internal tick (1 kHz by default) regardless of how often the
//! desktop senses feed it or how often the renderer draws. Sensory input is
//! injected as a current on named neuron populations; motor output is read back
//! as a per-population firing rate.

use crate::connectome::Connectome;

/// Neuron model parameters, shared by every cell.
///
/// These are the values the port has to match against the original; they are
/// deliberately in one struct so a single edit re-tunes the whole brain.
#[derive(Clone, Copy, Debug)]
pub struct LifParams {
    /// Integration step, seconds. 1 ms gives the 1 kHz internal rate.
    pub dt: f32,
    /// Membrane time constant, seconds.
    pub tau_m: f32,
    /// Synaptic current decay time constant, seconds.
    pub tau_syn: f32,
    /// Resting / reset potential, in the same arbitrary units as the threshold.
    pub v_rest: f32,
    pub v_reset: f32,
    pub v_thresh: f32,
    /// Absolute refractory period, seconds.
    pub t_refrac: f32,
    /// Scales a raw synapse count into a current step.
    pub w_scale: f32,
    /// Axonal delay applied to every synapse, in ticks. One tick per synapse is
    /// what produces the multi-millisecond escape latency through the loom
    /// pathway rather than an instantaneous cascade.
    pub delay_ticks: u16,
}

impl Default for LifParams {
    fn default() -> Self {
        // PLACEHOLDER values: physiologically sane, but not yet reconciled with
        // the original Sim.swift. See README "porting status".
        Self {
            dt: 0.001,
            tau_m: 0.020,
            tau_syn: 0.005,
            v_rest: 0.0,
            v_reset: 0.0,
            v_thresh: 1.0,
            t_refrac: 0.002,
            w_scale: 0.01,
            delay_ticks: 1,
        }
    }
}

pub struct Sim {
    pub connectome: Connectome,
    pub params: LifParams,

    /// Membrane potential per neuron.
    v: Vec<f32>,
    /// Decaying synaptic current per neuron.
    i_syn: Vec<f32>,
    /// Ticks remaining in the refractory period, 0 = excitable.
    refrac: Vec<u16>,
    /// Externally injected current, cleared every tick after it is consumed.
    i_ext: Vec<f32>,

    /// Ring of pending synaptic currents, one slot per delay tick.
    delay_ring: Vec<Vec<f32>>,
    ring_head: usize,

    /// Neurons that fired on the tick just completed.
    spikes: Vec<u32>,
    /// Monotonic tick counter since [`Sim::new`] or the last [`Sim::reset`].
    tick: u64,
}

impl Sim {
    pub fn new(connectome: Connectome, params: LifParams) -> Self {
        let n = connectome.neuron_count();
        let ring_len = params.delay_ticks.max(1) as usize;
        Self {
            connectome,
            params,
            v: vec![params.v_rest; n],
            i_syn: vec![0.0; n],
            refrac: vec![0; n],
            i_ext: vec![0.0; n],
            delay_ring: vec![vec![0.0; n]; ring_len],
            ring_head: 0,
            spikes: Vec::new(),
            tick: 0,
        }
    }

    pub fn reset(&mut self) {
        self.v.fill(self.params.v_rest);
        self.i_syn.fill(0.0);
        self.refrac.fill(0);
        self.i_ext.fill(0.0);
        for slot in &mut self.delay_ring {
            slot.fill(0.0);
        }
        self.ring_head = 0;
        self.spikes.clear();
        self.tick = 0;
    }

    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Elapsed simulated time in seconds.
    pub fn time(&self) -> f64 {
        self.tick as f64 * self.params.dt as f64
    }

    /// Neurons that fired on the most recent [`Sim::step`].
    pub fn spikes(&self) -> &[u32] {
        &self.spikes
    }

    pub fn potentials(&self) -> &[f32] {
        &self.v
    }

    /// Add current to a neuron for the next tick only. Sensory pathways call
    /// this every tick for as long as the stimulus is present.
    pub fn inject(&mut self, neuron: u32, current: f32) {
        self.i_ext[neuron as usize] += current;
    }

    pub fn inject_all(&mut self, neurons: &[u32], current: f32) {
        for &n in neurons {
            self.i_ext[n as usize] += current;
        }
    }

    /// Advance one tick.
    pub fn step(&mut self) {
        let p = self.params;
        let n = self.v.len();
        self.spikes.clear();

        // Deliver the synaptic currents whose delay expires on this tick, then
        // free the slot for reuse `delay_ticks` from now.
        let arrived = &mut self.delay_ring[self.ring_head];
        for (syn, slot) in self.i_syn.iter_mut().zip(arrived.iter_mut()) {
            *syn += *slot;
            *slot = 0.0;
        }

        let decay_syn = (-p.dt / p.tau_syn).exp();
        let decay_mem = (-p.dt / p.tau_m).exp();

        for i in 0..n {
            let drive = self.i_syn[i] + self.i_ext[i];
            self.i_ext[i] = 0.0;

            if self.refrac[i] > 0 {
                self.refrac[i] -= 1;
                self.v[i] = p.v_reset;
            } else {
                // Exponential Euler: exact for the linear membrane equation at
                // this step size, so the 1 kHz tick stays stable.
                let v_inf = p.v_rest + drive;
                self.v[i] = v_inf + (self.v[i] - v_inf) * decay_mem;

                if self.v[i] >= p.v_thresh {
                    self.v[i] = p.v_reset;
                    self.refrac[i] = (p.t_refrac / p.dt).round() as u16;
                    self.spikes.push(i as u32);
                }
            }

            self.i_syn[i] *= decay_syn;
        }

        // Propagate this tick's spikes into the delayed-arrival ring.
        let slot = (self.ring_head + p.delay_ticks.max(1) as usize - 1) % self.delay_ring.len();
        for &pre in &self.spikes {
            let (targets, weights) = self.connectome.out_edges(pre as usize);
            let dst = &mut self.delay_ring[slot];
            for (&t, &w) in targets.iter().zip(weights) {
                dst[t as usize] += w * p.w_scale;
            }
        }

        self.ring_head = (self.ring_head + 1) % self.delay_ring.len();
        self.tick += 1;
    }

    /// Run `ticks` steps, calling `on_tick` after each one.
    pub fn run(&mut self, ticks: u64, mut on_tick: impl FnMut(&Sim)) {
        for _ in 0..ticks {
            self.step();
            on_tick(self);
        }
    }
}
