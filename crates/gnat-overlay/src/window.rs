//! An ordinary desktop window: one xdg-toplevel, painted in software.
//!
//! The brain view lives here rather than on a layer surface, because unlike the
//! fly it wants to be clicked, moved, and closed like any other window.
//!
//! It runs on its own Wayland connection so it can be driven from its own
//! thread, which is what the original does too — its brain panel renders on the
//! AppKit thread while the sim advances in the fly's render loop.

use anyhow::{Context, Result};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{
    Window as XdgWindow, WindowConfigure, WindowDecorations, WindowHandler,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::Flow;
use crate::canvas::Canvas;

#[derive(Clone, Debug)]
pub struct WindowConfig {
    pub title: String,
    pub app_id: String,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "gnat".into(),
            app_id: "io.github.lubabs770.gnat".into(),
            width: 480,
            height: 400,
        }
    }
}

/// A pointer press, in surface coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub x: f32,
    pub y: f32,
}

pub struct Window {
    queue: wayland_client::EventQueue<State>,
    state: State,
}

impl Window {
    pub fn new(config: WindowConfig) -> Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, queue) = registry_queue_init(&conn).context("initialising the registry")?;
        let qh = queue.handle();

        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor is not available")?;
        let xdg_shell = XdgShell::bind(&globals, &qh).context("xdg-shell is not available")?;
        let shm = Shm::bind(&globals, &qh).context("wl_shm is not available")?;

        let surface = compositor.create_surface(&qh);
        // Server-side decorations where the compositor offers them, so the
        // window looks like every other window on the desktop.
        let window = xdg_shell.create_window(surface, WindowDecorations::RequestServer, &qh);
        window.set_title(&config.title);
        window.set_app_id(&config.app_id);
        window.set_min_size(Some((240, 200)));
        // Mapped only after an initial commit with no buffer; the compositor
        // replies with a configure carrying the size it wants.
        window.commit();

        let pool = SlotPool::new(4096, &shm).context("creating the shm pool")?;

        let state = State {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            seat_state: SeatState::new(&globals, &qh),
            shm,
            pool,
            window,
            width: config.width,
            height: config.height,
            configured: false,
            exit: false,
            pointer: None,
            pointer_at: None,
            clicks: Vec::new(),
            start: std::time::Instant::now(),
            draw: None,
        };

        Ok(Self { queue, state })
    }

    /// Run until the draw callback returns [`Flow::Exit`] or the user closes
    /// the window. Clicks since the previous frame are handed to each call.
    pub fn run(&mut self, draw: impl FnMut(&mut Canvas, &[Click]) -> Flow + 'static) -> Result<()> {
        self.state.draw = Some(Box::new(draw));
        while !self.state.exit {
            self.queue
                .blocking_dispatch(&mut self.state)
                .context("dispatching Wayland events")?;
        }
        Ok(())
    }
}

type DrawFn = Box<dyn FnMut(&mut Canvas, &[Click]) -> Flow>;

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    pool: SlotPool,
    window: XdgWindow,
    width: u32,
    height: u32,
    configured: bool,
    exit: bool,
    pointer: Option<wl_pointer::WlPointer>,
    /// Where the pointer last was; a press carries no coordinates of its own.
    pointer_at: Option<(f64, f64)>,
    clicks: Vec<Click>,
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
        let clicks = std::mem::take(&mut self.clicks);
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
                    let flow = draw(&mut canvas, &clicks);

                    let surface = self.window.wl_surface();
                    surface.damage_buffer(0, 0, w as i32, h as i32);
                    surface.frame(qh, FrameCallbackData(surface.clone()));
                    if buffer.attach_to(surface).is_ok() {
                        self.window.commit();
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

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &XdgWindow) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &XdgWindow,
        configure: WindowConfigure,
        _: u32,
    ) {
        if let Some(w) = configure.new_size.0 {
            self.width = w.get();
        }
        if let Some(h) = configure.new_size.1 {
            self.height = h.get();
        }
        if !self.configured {
            self.configured = true;
            self.frame(qh);
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
        let ours = self.window.wl_surface();
        for e in events.iter().filter(|e| &e.surface == ours) {
            match e.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_at = Some(e.position);
                }
                PointerEventKind::Leave { .. } => self.pointer_at = None,
                PointerEventKind::Press { .. } => {
                    // A press event carries no position of its own, so use the
                    // last motion — which the compositor always sends first.
                    let (x, y) = self.pointer_at.unwrap_or(e.position);
                    self.clicks.push(Click {
                        x: x as f32,
                        y: y as f32,
                    });
                }
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
