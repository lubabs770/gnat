//! What the brain tells the body each frame, and how the sim's population
//! rates become those commands.
//!
//! Every field here is read off a real neuron population. The thresholds and
//! divisors are the original's, and they are the seam where "spikes" turns
//! into "behaviour" — the one place worth looking when the fly acts wrong but
//! the circuit tests pass.

use gnat_sim::Lif;

/// Body commands for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Signals {
    /// The giant fibre spiked: take off now.
    pub escape: bool,
    /// Looming-detector population rate, 0..1.
    pub nervous: f32,
    /// Steering, in rad/s, from the DNa01/DNa02 left-right rate difference.
    pub turn_bias: f32,
    /// An MDN burst: walk backwards.
    pub backward: bool,
    /// DNp09 forward-walking command rate, roughly 0..1.5.
    pub walk_drive: f32,
    /// DNg11 grooming command rate, roughly 0..1.5.
    pub groom_drive: f32,
    /// DNp02/04/11 escape-manoeuvre rate, roughly 0..1.3.
    pub wing_drive: f32,
    /// Whole-population activity, roughly 0..1.
    pub arousal: f32,
    /// Thermal scaling of locomotion. A hot machine is a fast fly.
    pub tempo: f32,
    /// Circadian plus idle: sleep-like state.
    pub sleep: bool,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            escape: false,
            nervous: 0.0,
            turn_bias: 0.0,
            backward: false,
            walk_drive: 0.0,
            groom_drive: 0.0,
            wing_drive: 0.0,
            arousal: 0.0,
            tempo: 1.0,
            sleep: false,
        }
    }
}

/// Turns population rates into body commands.
///
/// Shared by the render loop and the behaviour tests so both exercise the
/// identical mapping — a divergence here would make the tests worthless.
#[derive(Clone, Debug, Default)]
pub struct SignalBuilder {
    dna_baseline: f32,
}

impl SignalBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn make(&mut self, sim: &mut Lif, dt: f32) -> Signals {
        let rates = sim.rates();
        let diff = rates.dna_l - rates.dna_r;
        // Slow adaptation, tau about 8 s. The connectome has a persistent
        // left/right wiring asymmetry; adapting it out means steady-state
        // walking is straight and only *transient* DNa asymmetries — visual,
        // or a click — actually steer.
        self.dna_baseline += (diff - self.dna_baseline) * (dt / 8.0).min(1.0);

        Signals {
            escape: sim.consume_gf(),
            nervous: (rates.loom / 80.0).clamp(0.0, 1.0),
            turn_bias: ((diff - self.dna_baseline) * 0.04).clamp(-1.0, 1.0),
            backward: rates.mdn > 8.0,
            walk_drive: (rates.fwd / 10.0).clamp(0.0, 1.3),
            groom_drive: rates.groom / 8.0,
            wing_drive: (rates.escw / 10.0).clamp(0.0, 1.3),
            arousal: (rates.pop / 20.0).clamp(0.0, 1.0),
            ..Signals::default()
        }
    }
}
