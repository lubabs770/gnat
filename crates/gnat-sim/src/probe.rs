//! Reading firing rates back out of the sim.
//!
//! Motor output is not a single spike but a rate over a short window, so the
//! body model gets a smooth signal instead of a stutter.

use crate::lif::Sim;

/// Sliding-window spike counter over a fixed neuron population.
pub struct RateProbe {
    population: Vec<u32>,
    /// Per-tick spike counts, oldest entry at `head`.
    window: Vec<u16>,
    head: usize,
    total: u32,
    dt: f32,
}

impl RateProbe {
    /// `window_ticks` is the averaging window; at the default 1 kHz tick, 50
    /// ticks is a 50 ms window.
    pub fn new(mut population: Vec<u32>, window_ticks: usize, dt: f32) -> Self {
        // Sorted so `observe` can binary-search it.
        population.sort_unstable();
        population.dedup();
        Self {
            population,
            window: vec![0; window_ticks.max(1)],
            head: 0,
            total: 0,
            dt,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.population.is_empty()
    }

    /// Call once after every [`Sim::step`].
    pub fn observe(&mut self, sim: &Sim) {
        let count = sim
            .spikes()
            .iter()
            .filter(|s| self.population.binary_search(s).is_ok())
            .count() as u16;

        self.total -= self.window[self.head] as u32;
        self.window[self.head] = count;
        self.total += count as u32;
        self.head = (self.head + 1) % self.window.len();
    }

    /// Mean firing rate across the population, in spikes per second.
    pub fn rate_hz(&self) -> f32 {
        if self.population.is_empty() {
            return 0.0;
        }
        let window_s = self.window.len() as f32 * self.dt;
        self.total as f32 / (self.population.len() as f32 * window_s)
    }

    /// True once any neuron in the population has fired inside the window.
    pub fn active(&self) -> bool {
        self.total > 0
    }
}
