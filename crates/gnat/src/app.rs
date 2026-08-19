//! The coordinator: senses in, spikes in the middle, a fly on the screen.
//!
//! This is the only place where the four crates meet, and the only place that
//! knows about wall-clock pacing. The order inside a frame matters and mirrors
//! the original: sense the world, drive the circuit, step it in whole
//! milliseconds, read the population rates as body commands, move the body,
//! draw.

use anyhow::{Context, Result};
use gnat_body::{Fly, Ledge as SceneLedge, SignalBuilder, Signals, circadian};
use gnat_overlay::{Canvas, Config, Flow, Overlay};
use gnat_senses::{Activity, Event, EventStream, Hypr, Thermal, ledges};
use gnat_sim::{BrainPoints, Circuit, Lif};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant, SystemTime};

use crate::brain::{BrainData, SpikeBus, StimQueue};
use crate::control::{self, Control};
use crate::coord::Frame;
use crate::render;

/// How often the window list is re-read. The original polls at about 1.4 Hz;
/// the events socket covers anything that actually changed in between.
const TERRAIN_INTERVAL: Duration = Duration::from_millis(700);
/// How often temperature and the clock are re-read. Neither moves quickly.
const AMBIENT_INTERVAL: Duration = Duration::from_secs(2);
/// The sim can fall at most this far behind before frames are dropped rather
/// than the loop spiralling.
const MAX_STEPS_PER_FRAME: i64 = 50;

/// Temperature, in °C, mapped onto the original's thermal-state range. Linux
/// gives a real number where macOS gave four buckets, so this interpolates
/// where the original stepped.
const COOL_C: f32 = 45.0;
const HOT_C: f32 = 90.0;
const TEMPO_COOL: f32 = 1.0;
const TEMPO_HOT: f32 = 1.5;

pub struct App {
    hypr: Hypr,
    events: Receiver<Event>,
    sim: Lif,
    builder: SignalBuilder,
    fly: Fly,
    thermal: Thermal,
    activity: Activity,
    utc_offset_secs: i32,

    terrain: Vec<SceneLedge>,
    last_frame: Instant,
    last_terrain: Instant,
    last_ambient: Instant,

    mouse: Option<(f32, f32)>,
    prev_mouse: Option<(f32, f32)>,
    mouse_vel: (f32, f32),
    window_loom: (f32, f32),
    ms_accumulator: f64,

    tempo: f32,
    sleepy: bool,
    circadian_activity: f32,

    /// Set once the first configure has told us the real output size.
    frame: Frame,

    /// Spikes out to the brain view, stimulation back in. Both are `None` when
    /// no view is open, so nothing is paid for a window that is not there.
    spikes: Option<Arc<SpikeBus>>,
    stims: Option<Arc<StimQueue>>,

    control: Control,
    /// A deliberate scare, decaying like any other looming stimulus.
    loom_override: f32,
}

impl App {
    pub fn new(circuit: Circuit, seed: u64) -> Result<Self> {
        let hypr = Hypr::connect()?;
        let monitor = hypr
            .monitors()?
            .into_iter()
            .next()
            .context("Hyprland reports no outputs")?;
        let frame = Frame::new(monitor.width as u32, monitor.height as u32);

        // The event socket blocks, so it gets its own thread and a channel.
        let (tx, events) = std::sync::mpsc::channel();
        let stream = EventStream::connect()?;
        std::thread::spawn(move || {
            for event in stream {
                if tx.send(event).is_err() {
                    break;
                }
            }
        });

        let now = Instant::now();
        let mut fly = Fly::new((0.0, 0.0), seed);
        fly.pos = (frame.width * 0.2, -frame.height * 0.2);

        Ok(Self {
            hypr,
            events,
            sim: Lif::new(circuit, seed),
            builder: SignalBuilder::new(),
            fly,
            thermal: Thermal::discover(),
            activity: Activity::new(Duration::from_secs(600)),
            utc_offset_secs: local_utc_offset(),
            terrain: Vec::new(),
            last_frame: now,
            last_terrain: now - TERRAIN_INTERVAL,
            last_ambient: now - AMBIENT_INTERVAL,
            mouse: None,
            prev_mouse: None,
            mouse_vel: (0.0, 0.0),
            window_loom: (0.0, 0.0),
            ms_accumulator: 0.0,
            tempo: 1.0,
            sleepy: false,
            circadian_activity: 1.0,
            frame,
            spikes: None,
            stims: None,
            control: Control::default(),
            loom_override: 0.0,
        })
    }

    /// The shared control state, for the socket server to read and write.
    pub fn control(&self) -> Control {
        self.control.clone()
    }

    /// Attach a brain view's channels. Turns on spike sampling, which is pure
    /// overhead while nothing is drawing.
    pub fn attach_brain(&mut self, spikes: Arc<SpikeBus>, stims: Arc<StimQueue>) {
        self.sim.set_spike_logging(true);
        self.spikes = Some(spikes);
        self.stims = Some(stims);
    }

    /// The fly, for callers that need to look at it — the snapshot tool
    /// centres its crop on wherever it currently is.
    pub fn fly(&self) -> &Fly {
        &self.fly
    }

    /// The current output frame.
    pub fn output(&self) -> Frame {
        self.frame
    }

    /// Walkable edges the fly currently knows about.
    pub fn terrain(&self) -> &[SceneLedge] {
        &self.terrain
    }

    /// One frame: sense, simulate, move, draw.
    pub fn frame(&mut self, canvas: &mut Canvas) -> Flow {
        // The compositor owns the surface size, so take it from the canvas
        // rather than assuming the output never changes.
        if canvas.width as f32 != self.frame.width || canvas.height as f32 != self.frame.height {
            self.frame = Frame::new(canvas.width, canvas.height);
            self.terrain.clear();
        }

        let now = Instant::now();
        // Clamped: a stalled compositor must not teleport the fly.
        let dt = (now - self.last_frame).as_secs_f32().clamp(0.0, 0.05);
        self.last_frame = now;

        let paused = match self.take_commands() {
            Some(flow) => return flow,
            None => self.control.lock().unwrap().paused,
        };
        if paused {
            // Keep drawing so the fly does not vanish, but stop time for it.
            canvas.clear();
            render::draw_fly(canvas, &self.fly, &self.frame);
            return Flow::Continue;
        }

        self.drain_events();
        self.poll_cursor();
        if now - self.last_terrain >= TERRAIN_INTERVAL {
            self.last_terrain = now;
            self.poll_terrain();
        }
        if now - self.last_ambient >= AMBIENT_INTERVAL {
            self.last_ambient = now;
            self.poll_ambient();
        }
        self.activity.tick(Duration::from_secs_f32(dt));

        let signals = self.step_brain(dt);
        self.fly.terrain = self.terrain.clone();
        self.fly
            .update(dt, self.frame.bounds(), self.mouse, Some(signals));

        canvas.clear();
        render::draw_fly(canvas, &self.fly, &self.frame);
        self.publish_status();
        Flow::Continue
    }

    /// Apply anything the control socket asked for. Returns a flow when the
    /// answer is "stop".
    fn take_commands(&mut self) -> Option<Flow> {
        let mut c = self.control.lock().unwrap();
        if c.quit {
            return Some(Flow::Exit);
        }
        if c.scare {
            c.scare = false;
            drop(c);
            // The same magnitude the original's "scare all" uses: a real
            // stimulus into the real circuit, not a scripted takeoff.
            self.loom_override = 0.6;
        }
        None
    }

    fn publish_status(&self) {
        let mut c = self.control.lock().unwrap();
        c.state = format!("{:?}", self.fly.state).to_lowercase();
        c.pop_hz = self.sim.rates().pop;
        c.neurons = self.sim.len();
        c.ledges = self.terrain.len();
        c.sleeping = self.sleepy;
    }

    fn drain_events(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    // Any compositor event at all is evidence of a human.
                    self.activity.poke();
                    match event {
                        // A window appearing near the fly is a real looming
                        // object; the terrain poll will pick up its geometry,
                        // and until then the startle is what matters.
                        Event::OpenWindow { .. } => self.inject_window_loom(0.55),
                        // Clicks are not visible to a Wayland client, so a
                        // focus change stands in for a tap on the substrate.
                        Event::ActiveWindow { .. } => {
                            let sens = self.sim.groups().sens.clone();
                            self.sim.stimulate(&sens, 0.2, 130);
                        }
                        Event::CloseWindow { .. } | Event::MoveWindow { .. } => {
                            // Force a terrain refresh: the fly may be standing
                            // on what just moved or vanished.
                            self.last_terrain = Instant::now() - TERRAIN_INTERVAL;
                        }
                        _ => {}
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn poll_cursor(&mut self) {
        if let Ok(p) = self.hypr.cursor_pos() {
            let scene = self.frame.to_scene(p.x as f32, p.y as f32);
            if self.mouse != Some(scene) {
                self.activity.poke();
            }
            self.mouse = Some(scene);
        }
    }

    fn poll_terrain(&mut self) {
        let Ok(clients) = self.hypr.clients() else {
            return;
        };
        let screen = ledges(&clients, self.frame.width as i32, self.frame.height as i32);
        self.terrain = self.frame.ledges_to_scene(&screen);
    }

    fn poll_ambient(&mut self) {
        self.tempo = match self.thermal.hottest_c() {
            Some(c) => {
                let t = ((c - COOL_C) / (HOT_C - COOL_C)).clamp(0.0, 1.0);
                TEMPO_COOL + (TEMPO_HOT - TEMPO_COOL) * t
            }
            None => 1.0,
        };
        let hour = gnat_senses::circadian::local_hour(SystemTime::now(), self.utc_offset_secs);
        self.circadian_activity = circadian::activity(hour);
        let idle = self.activity.idle_for().as_secs_f32();
        // Night plus a long idle, or a very long idle at any hour.
        self.sleepy = (idle > 600.0 && !(6.0..22.0).contains(&hour)) || idle > 1800.0;
    }

    /// Cursor kinematics into looming drive for each eye, plus an air puff.
    ///
    /// This is the sensory transduction step. Everything downstream of the
    /// LC4/LPLC2 population is the real connectome; everything here is a
    /// modelling choice about what a cursor *is* to a fly.
    fn compute_loom(&mut self, dt: f32) -> (f32, f32, f32) {
        let Some(m) = self.mouse else {
            return (0.0, 0.0, 0.0);
        };
        if let Some(pm) = self.prev_mouse
            && dt > 0.0
        {
            let v = ((m.0 - pm.0) / dt, (m.1 - pm.1) / dt);
            self.mouse_vel.0 += (v.0 - self.mouse_vel.0) * 0.4;
            self.mouse_vel.1 += (v.1 - self.mouse_vel.1) * 0.4;
        }
        self.prev_mouse = Some(m);

        let rel = (m.0 - self.fly.pos.0, m.1 - self.fly.pos.1);
        let dist = (rel.0 * rel.0 + rel.1 * rel.1).sqrt().max(20.0);
        // Radial approach speed; positive means the cursor is closing in.
        let approach = -(rel.0 * self.mouse_vel.0 + rel.1 * self.mouse_vel.1) / dist;
        let mut loom =
            (approach / dist * 6.0).clamp(0.0, 1.0) * (1.0 - dist / 800.0).clamp(0.0, 1.0);
        // Hovering close counts too: a big stationary object is still a threat.
        loom += ((130.0 - dist) / 130.0).clamp(0.0, 1.0) * 0.5;
        let loom = (loom + self.loom_override).clamp(0.0, 1.0);

        // Split between the eyes by bearing relative to the fly's heading.
        let (sin, cos) = self.fly.heading.sin_cos();
        let rd = (rel.0 / dist, rel.1 / dist);
        let cross_z = cos * rd.1 - sin * rd.0; // positive: threat on the left
        let lw = (0.5 + 0.5 * cross_z).clamp(0.12, 1.0);
        let rw = (0.5 - 0.5 * cross_z).clamp(0.12, 1.0);

        let speed = (self.mouse_vel.0.powi(2) + self.mouse_vel.1.powi(2)).sqrt();
        let puff = (speed / 1500.0).clamp(0.0, 1.0) * (1.0 - dist / 500.0).clamp(0.0, 1.0);
        (loom * lw, loom * rw, puff)
    }

    fn inject_window_loom(&mut self, strength: f32) {
        // Direction is unknown until the next terrain poll, so the startle is
        // delivered to both eyes; a real bearing would need geometry we do not
        // have yet at event time.
        self.window_loom.0 = self.window_loom.0.max(strength);
        self.window_loom.1 = self.window_loom.1.max(strength);
    }

    fn step_brain(&mut self, dt: f32) -> Signals {
        // Clicks in the brain view arrive on its own thread; deliver them here,
        // where the sim is actually owned.
        if let Some(stims) = &self.stims {
            for stim in stims.drain() {
                self.sim
                    .stimulate(&stim.neurons, stim.strength, stim.duration_ms);
            }
        }

        let (loom_l, loom_r, puff) = self.compute_loom(dt);

        let decay = (-4.0 * dt).exp();
        self.window_loom.0 *= decay;
        self.window_loom.1 *= decay;
        self.loom_override = (self.loom_override - dt * 1.2).max(0.0);

        self.sim.inputs.loom_l = loom_l.max(self.window_loom.0);
        self.sim.inputs.loom_r = loom_r.max(self.window_loom.1);
        self.sim.inputs.air_puff = puff.max(self.activity.vibration() * 0.30);
        // Body back into brain: leg proprioception from the current gait.
        self.sim.inputs.gait_drive = self.fly.walking_intensity();
        self.sim.inputs.gait_phase = self.fly.gait_phase();
        // Circadian and sleep neuromodulation, compressed toward 1. The LIF
        // neurons sit just below threshold, so a raw multiplier silences them
        // outright: a siesta should mean "less active", not comatose.
        self.sim.inputs.activity_scale =
            (1.0 - (1.0 - self.circadian_activity) * 0.35) * if self.sleepy { 0.75 } else { 1.0 };
        self.sim.inputs.sensory_gate = if self.sleepy { 0.55 } else { 1.0 };

        // Step in whole milliseconds, carrying the remainder, so the 1 kHz
        // internal rate stays honest regardless of frame pacing.
        self.ms_accumulator += dt as f64 * 1000.0;
        let steps = (self.ms_accumulator as i64).min(MAX_STEPS_PER_FRAME);
        self.ms_accumulator -= steps as f64;
        self.sim.step(steps);

        if let Some(bus) = &self.spikes {
            bus.push(&self.sim.drain_spikes());
        }

        Signals {
            tempo: self.tempo,
            sleep: self.sleepy,
            ..self.builder.make(&mut self.sim, dt)
        }
    }
}

/// The local UTC offset in seconds.
///
/// `std::time` has no notion of local time and pulling in a date library for
/// one number is not worth it, so this asks `date` once at startup. A failure
/// falls back to UTC, which shifts the fly's bedtime rather than breaking it.
fn local_utc_offset() -> i32 {
    let Ok(out) = std::process::Command::new("date").arg("+%z").output() else {
        return 0;
    };
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    parse_utc_offset(&raw).unwrap_or(0)
}

/// Parse `+0200` / `-0530` into seconds.
fn parse_utc_offset(raw: &str) -> Option<i32> {
    let bytes = raw.as_bytes();
    if bytes.len() != 5 {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i32 = raw.get(1..3)?.parse().ok()?;
    let minutes: i32 = raw.get(3..5)?.parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

pub fn run(circuit: Circuit, points: Option<BrainPoints>, seed: u64) -> Result<()> {
    let mut app = App::new(circuit.clone(), seed)?;

    // The brain view gets its own thread and its own Wayland connection, so a
    // slow repaint there cannot stall the fly.
    if let Some(points) = points {
        let data = Arc::new(BrainData::new(&circuit, points));
        let spikes = Arc::new(SpikeBus::default());
        let stims = Arc::new(StimQueue::default());
        app.attach_brain(spikes.clone(), stims.clone());
        std::thread::spawn(move || {
            if let Err(e) = crate::brain::run(data, spikes, stims) {
                eprintln!("brain view: {e}");
            }
        });
    }

    control::serve(app.control());

    let mut overlay = Overlay::new(Config {
        namespace: "gnat".into(),
        ..Config::default()
    })?;
    overlay.run(move |canvas| app.frame(canvas))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_offsets_parse() {
        assert_eq!(parse_utc_offset("+0000"), Some(0));
        assert_eq!(parse_utc_offset("+0200"), Some(7200));
        assert_eq!(parse_utc_offset("-0530"), Some(-19800));
        assert_eq!(parse_utc_offset("-0400"), Some(-14400));
    }

    #[test]
    fn nonsense_offsets_are_rejected_rather_than_guessed() {
        assert_eq!(parse_utc_offset(""), None);
        assert_eq!(parse_utc_offset("0200"), None);
        assert_eq!(parse_utc_offset("+02:00"), None);
        assert_eq!(parse_utc_offset("+02xx"), None);
    }

    #[test]
    fn the_live_machine_reports_a_sane_offset() {
        let secs = local_utc_offset();
        assert!(
            (-12 * 3600..=14 * 3600).contains(&secs),
            "{secs} is not a real time zone"
        );
    }
}
