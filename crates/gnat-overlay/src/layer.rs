//! The window the fly lives in: a `wlr-layer-shell` surface that floats above
//! every normal window and, by default, lets every click straight through.
//!
//! On macOS the original had to fake this. On Wayland it is a first-class
//! protocol feature: a surface whose *input region* is empty receives no
//! pointer or touch events at all, and the compositor delivers them to whatever
//! is underneath instead. See [`Overlay::click_through`].
//!
//! The brain view is deliberately *not* built on this — it is an ordinary
//! xdg-toplevel, because it wants clicks.

use crate::Flow;
use crate::canvas::Canvas;
use anyhow::{Context, Result};
use smithay_client_toolkit::compositor::{
    CompositorHandler, CompositorState, FrameCallbackData, Region,
};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

pub use smithay_client_toolkit::shell::wlr_layer::Layer as ShellLayer;

/// How the overlay should be created.
#[derive(Clone, Debug)]
pub struct Config {
    /// The layer-shell namespace. Shows up in `hyprctl layers`, and is what
    /// Hyprland's `layerrule` matches on.
    pub namespace: String,
    /// Which layer to sit on. `Overlay` draws above everything, including
    /// fullscreen windows.
    pub layer: ShellLayer,
    /// Install an empty input region, so clicks, drags and scrolls pass
    /// through to the window underneath. Almost always what you want; the
    /// escape hatch exists so the passthrough can be tested against a control.
    pub click_through: bool,
    /// Restrict to one output by name (`HDMI-A-2`). `None` lets the compositor
    /// choose, which is what you want for a single-monitor setup.
    pub output: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            namespace: "gnat".into(),
            layer: ShellLayer::Overlay,
            click_through: true,
            output: None,
        }
    }
}

/// A live layer-shell overlay.
/// Pumps the registry far enough to learn what the outputs are called.
struct OutputProbe {
    registry_state: RegistryState,
    output_state: OutputState,
}

impl OutputHandler for OutputProbe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for OutputProbe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_registry!(OutputProbe);
smithay_client_toolkit::delegate_dispatch2!(OutputProbe);

/// Find an output by name, e.g. `HDMI-A-2`.
///
/// Proxies belong to the connection rather than to a queue, so an output found
/// here is perfectly usable by the real surface afterwards.
fn find_output(conn: &Connection, name: &str) -> Result<wl_output::WlOutput> {
    let (globals, mut queue) =
        registry_queue_init::<OutputProbe>(conn).context("initialising an output probe")?;
    let qh = queue.handle();
    let mut probe = OutputProbe {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
    };
    // The first roundtrip delivers the wl_output globals; the second delivers
    // the name and mode events that describe them.
    queue.roundtrip(&mut probe)?;
    queue.roundtrip(&mut probe)?;

    let mut seen = Vec::new();
    for output in probe.output_state.outputs() {
        match probe.output_state.info(&output).and_then(|i| i.name) {
            Some(n) if n == name => return Ok(output),
            Some(n) => seen.push(n),
            None => {}
        }
    }
    anyhow::bail!("no output named {name}; this compositor reports {seen:?}")
}

pub struct Overlay {
    conn: Connection,
    queue: wayland_client::EventQueue<State>,
    state: State,
}

impl Overlay {
    pub fn new(config: Config) -> Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, queue) = registry_queue_init(&conn).context("initialising the registry")?;
        let qh = queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor is not available")?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .context("zwlr_layer_shell_v1 is not available; is this compositor wlroots-based?")?;
        let shm = Shm::bind(&globals, &qh).context("wl_shm is not available")?;
        let output_state = OutputState::new(&globals, &qh);

        // Output names arrive as events, not as globals, so the registry has to
        // be pumped before any of them can be matched. Doing that on a throwaway
        // queue keeps the main state's construction in one piece.
        let output = match &config.output {
            Some(name) => Some(find_output(&conn, name)?),
            None => None,
        };

        let surface = compositor.create_surface(&qh);

        if config.click_through {
            // An empty region means "no part of this surface accepts input".
            // This is the whole trick, and it is one call.
            let region = Region::new(&compositor).context("creating an empty input region")?;
            surface.set_input_region(Some(region.wl_region()));
            // The compositor copies the region on commit, so dropping it here
            // is safe and keeps the object from leaking.
        }

        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            config.layer,
            Some(config.namespace.clone()),
            output.as_ref(),
        );
        // Span the whole output: anchoring to all four edges makes the
        // compositor pick the size for us.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        // Reserve no space and do not push tiled windows around.
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // The surface is only mapped after an initial commit with no buffer;
        // the compositor answers with a configure carrying the real size.
        layer.commit();

        let pool = SlotPool::new(4096, &shm).context("creating the shm pool")?;

        let state = State {
            registry_state: RegistryState::new(&globals),
            output_state,
            seat_state: SeatState::new(&globals, &qh),
            shm,
            pool,
            layer,
            width: 0,
            height: 0,
            configured: false,
            exit: false,
            pointer: None,
            pointer_enters: 0,
            pointer_buttons: 0,
            start: std::time::Instant::now(),
            draw: None,
        };

        Ok(Self { conn, queue, state })
    }

    /// Size the compositor gave us, once configured.
    pub fn size(&self) -> (u32, u32) {
        (self.state.width, self.state.height)
    }

    /// How many times the pointer has entered this surface.
    ///
    /// With `click_through` set this must stay at zero no matter where the
    /// cursor goes, which is exactly how the passthrough is verified.
    pub fn pointer_enters(&self) -> u32 {
        self.state.pointer_enters
    }

    /// How many button presses this surface has received.
    pub fn pointer_buttons(&self) -> u32 {
        self.state.pointer_buttons
    }

    /// Run the overlay, calling `draw` once per frame, until it returns
    /// [`Flow::Exit`] or the compositor closes the surface.
    pub fn run(&mut self, draw: impl FnMut(&mut Canvas) -> Flow + 'static) -> Result<()> {
        self.state.draw = Some(Box::new(draw));
        while !self.state.exit {
            self.queue
                .blocking_dispatch(&mut self.state)
                .context("dispatching Wayland events")?;
        }
        Ok(())
    }

    /// Process any pending events without blocking. For callers driving their
    /// own loop rather than handing it to [`Overlay::run`].
    pub fn pump(&mut self) -> Result<()> {
        self.queue.flush()?;
        if let Some(guard) = self.queue.prepare_read() {
            let _ = guard.read();
        }
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

type DrawFn = Box<dyn FnMut(&mut Canvas) -> Flow>;

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    width: u32,
    height: u32,
    configured: bool,
    exit: bool,
    pointer: Option<wl_pointer::WlPointer>,
    pointer_enters: u32,
    pointer_buttons: u32,
    start: std::time::Instant,
    draw: Option<DrawFn>,
}

impl State {
    fn frame(&mut self, qh: &QueueHandle<Self>) {
        let (w, h) = (self.width, self.height);
        if w == 0 || h == 0 {
            return;
        }
        let Some(mut draw) = self.draw.take() else {
            return;
        };

        let stride = w as i32 * 4;
        let flow =
            match self
                .pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            {
                Ok((buffer, pixels)) => {
                    let mut canvas = Canvas {
                        width: w,
                        height: h,
                        pixels,
                        time_ms: self.start.elapsed().as_millis() as u64,
                    };
                    let flow = draw(&mut canvas);

                    let surface = self.layer.wl_surface();
                    surface.damage_buffer(0, 0, w as i32, h as i32);
                    surface.frame(qh, FrameCallbackData(surface.clone()));
                    if buffer.attach_to(surface).is_ok() {
                        self.layer.commit();
                    }
                    flow
                }
                Err(_) => Flow::Exit,
            };

        self.draw = Some(draw);
        if matches!(flow, Flow::Exit) {
            self.exit = true;
        }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame(qh);
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            self.width = w;
            self.height = h;
        }
        if !self.configured {
            self.configured = true;
            self.frame(qh);
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        // The pointer is bound purely so passthrough can be observed: with an
        // empty input region it must never deliver an event for our surface.
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let ours = self.layer.wl_surface();
        for e in events.iter().filter(|e| &e.surface == ours) {
            match e.kind {
                PointerEventKind::Enter { .. } => self.pointer_enters += 1,
                PointerEventKind::Press { .. } => self.pointer_buttons += 1,
                _ => {}
            }
        }
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
smithay_client_toolkit::delegate_dispatch2!(State);
