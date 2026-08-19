# gnat

A fruit fly, simulated from its connectome, living on your Hyprland desktop.

A Linux/Wayland port of **DesktopFly** (macOS). The fly walks along the top
edges of your windows, startles when the cursor lunges at it, sleeps at night,
and speeds up when the laptop gets hot — and every one of those behaviours comes
out of a leaky integrate-and-fire simulation of a real *Drosophila* connectome,
not a state machine.

## Status

Early. The desktop sense layer is **working and verified against a live
Hyprland 0.56.2**; the simulation core is **scaffolded but not yet reconciled
with the original**. See [porting status](#porting-status).

```
cargo test --workspace                        # 27 tests, all green
cargo run -p gnat --bin gnat -- --senses      # one reading from every sense
cargo run -p gnat-senses --example probe      # live 20s dump of the whole sense layer
```

## Layout

| Crate | What it is | State |
|---|---|---|
| `gnat-sim` | LIF engine, CSR connectome, `.gnat` file format. No OS dependencies at all. | scaffolded |
| `gnat-senses` | Hyprland IPC, window terrain, looming, thermal, circadian, activity. | working |
| `gnat-overlay` | The click-through `wlr-layer-shell` surface. | not started |
| `gnat` | The binary that wires them together. | stub |
| `etl/etl.py` | FlyWire CSV exports → `.gnat`. | placeholder |

## How the macOS senses map onto Wayland

macOS handed DesktopFly permission-free access to the global cursor, every
window frame, and system idle state. Hyprland gives you most of that too, through
different channels — and one piece is genuinely constrained.

| Sense | macOS | Here | Fidelity |
|---|---|---|---|
| Window terrain, looms, closing underfoot | Accessibility API | Hyprland `.socket2.sock` events + `j/clients` | **Better.** Structured JSON, and a real event stream. |
| Global cursor (the escape trigger) | Continuous event tap | `j/cursorpos` polled at 60 Hz over `.socket.sock` | **Degraded but fine.** See below. |
| Typing as substrate vibration | Idle-time API | `ext-idle-notify-v1` (planned); activity proxy today | **Equal,** and content-blind by protocol rather than by promise. |
| Circadian rhythm | Wall clock | Wall clock | Identical. |
| Machine temperature | Thermal-state enum | `/sys/class/hwmon` in °C | **Better.** Real numbers, not three buckets. |
| Clicks as taps | Global event tap | — | **Lost.** Downgraded to window/focus events. |
| Menu bar item | `NSStatusItem` | Waybar module or a control socket + CLI | Reworked. |

### The cursor

Wayland deliberately withholds continuous global pointer position from a client
whose surface the pointer is not over — and the fly's overlay has an *empty
input region*, so it never receives pointer events at all. That is by design and
not a bug to route around.

So the cursor is polled: `j/cursorpos` on Hyprland's request socket, at 60 Hz,
**measured at 60.5 Hz on this machine** with no process spawn (talking to the
socket directly rather than shelling out to `hyprctl` is what makes that
affordable).

This costs less than it sounds like. The sim runs its own 1 kHz internal tick;
sensory input does not have to match it, and `CursorTracker` interpolates
velocity between polls. The escape latency lives inside the connectome, not in
the input rate, so it survives intact.

### Clicks

There is no honest way to get global button events as a normal Wayland client,
and Hyprland does not expose them over IPC. Rather than pretend, this sense is
downgraded: taps are derived from window open/close and focus changes. It is the
one macOS capability with no Wayland analog, and it is a security-model
decision, not an engineering gap.

## Porting status

What is real, and what is still a placeholder, stated plainly:

**Real and live-verified** — the whole of `gnat-senses`. Hyprland event parsing
(including window titles containing commas), `j/clients` / `j/cursorpos` /
`j/monitors`, front-to-back window sorting, ledge extraction with occlusion,
loom drive, thermal discovery across 17 sensors, circadian phase.

**Real but unreconciled** — `gnat-sim`. The LIF integrator, the CSR connectome,
the `.gnat` format and the rate probes are all written and tested. What is *not*
settled is `LifParams`: `tau_m`, `v_thresh`, `w_scale`, `delay_ticks` and the
rest are physiologically sane placeholders, **not** the original's values.

**Placeholder** — `etl/etl.py`. It builds a valid `.gnat` from FlyWire Codex CSV
exports and round-trips cleanly into the Rust loader, but the neuron subset and
the weight scaling are guesses at what the original selected.

### What is needed to finish the sim

The original's `Sim.swift` and `etl.py`. Specifically:

1. **`LifParams` values** — the six constants above.
2. **The neuron subset** — which FlyWire cells make up the 23,210, and how they
   were selected.
3. **The sensory and motor bindings** — which populations the loom pathway
   drives, and which the gait model reads.
4. **The `--simtest` thresholds** — the documented invariants, with numbers.

Until (4) arrives, `crates/gnat-sim/tests/invariants.rs` asserts the *mechanism*
rather than the biological figures — that the brain is silent at rest, that
stimulus reaches the motor end, that latency grows with path length and stays
bounded, that the refractory period caps firing rate, and that the whole thing
is deterministic. Every one of those tests is written so that adopting the real
numbers is a constant change, not a rewrite.

## Milestones

1. ~~Desktop sense layer against live Hyprland.~~ **Done**, ahead of plan.
2. **Click-through layer-shell overlay.** `zwlr_layer_surface_v1` on the
   `overlay` layer, anchored to all four edges, `set_exclusive_zone(-1)`, and an
   empty `wl_surface.set_input_region` so clicks pass through. This is the other
   genuinely-uncertain piece and should be proven before anything is drawn.
3. **Reconcile the sim** against the original, and run the real connectome.
4. Fly body and gait; render into the overlay.
5. Brain view — an ordinary xdg-toplevel, because it wants clicks.
6. Control surface: a Waybar module or a socket plus `gnat pause` / `gnat add`.

## Data

`data/` is empty in git. To populate it, export `connections.csv`,
`classification.csv` and `coordinates.csv` from FlyWire Codex into `data/raw/`,
then:

```
python3 etl/etl.py --data-dir data/raw --out data/brain.gnat
cargo run -p gnat-sim --example dump -- data/brain.gnat
```
