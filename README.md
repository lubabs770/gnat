# gnat

A fruit fly, simulated from its connectome, living on your Hyprland desktop.

A Linux/Wayland port of [**DesktopFly**](https://github.com/DenisSergeevitch/desktop-fly)
by Denis Shiryaev (MIT). The fly walks along the top edges of your windows,
startles when the cursor lunges at it, sleeps at night, and speeds up when the
laptop gets hot — and every one of those behaviours falls out of a
leaky integrate-and-fire simulation of a real *Drosophila* connectome, not a
state machine.

## Status

The **simulation core is ported and passing the original's own invariants**,
the **desktop sense layer is working against live Hyprland 0.56.2**, and the
**click-through overlay is proven**. What is left is the fly itself: body,
gait, and the brain view.

```
cargo test --workspace                          # 38 tests
cargo run --release -p gnat -- --simtest        # circuit invariants, on real data
cargo run -p gnat -- --senses                   # one reading from every desktop sense
cargo run -p gnat-senses --example probe        # live 20s dump of the whole sense layer
cargo run --release -p gnat -- --overlay-test   # measure the click-through claim
```

`--simtest` on this machine:

```
circuit: 668 neurons | loom L/R: 162/152 | GF: 2 | DNa L/R: 2/2 | MDN: 4 | DNp09: 2 | DNg11: 6 | escW: 6 | ascend: 27 | sens: 16
spontaneous 4s: pop 5.48 Hz/neuron, LC 0.0 Hz, DNa02 L/R 9.4/5.7 Hz, MDN 0.3 Hz, GF spikes: 0
abrupt loom 0.4s: LC rate 180.4 Hz, GF spikes 2, first at 4 ms
behavior 20s: walk-drive on 26%, groom-drive on 8%, DNp09 0.0-10.9 Hz, pop 6.2 Hz
siesta 15s (scale 0.84): walk-drive on 20%
air puff 1s: GF spikes 20
left-eye loom: DNa L-R rate diff +4.5 -> +1.3 Hz, LC 31.3 Hz
click probes: GF cluster -> spike yes, DNg11 cluster -> groom rate 195 Hz
PASS: GF silent at rest, fires on loom; locomotor drive fluctuates; stim works; siesta alive
```

All four documented invariants reproduce: giant fibre silent over 4 s of rest,
fires **4 ms** after an abrupt loom (bound is ~10 ms), walk-drive duty 26%
(band is 20–50%), siesta duty 20% (floor is 3%).

## Layout

| Crate | What it is | State |
|---|---|---|
| `gnat-sim` | LIF circuit sim, `circuit.json` loader, seeded RNG, the invariant suite. No OS dependencies. | **ported** |
| `gnat-senses` | Hyprland IPC, window terrain, looming, thermal, circadian, activity. | **working** |
| `gnat-overlay` | The click-through `wlr-layer-shell` surface, and a software canvas. | **working** |
| `gnat` | The binary that wires them together. | 4 subcommands |

## The overlay

A `zwlr_layer_surface_v1` on the `overlay` layer, anchored to all four edges so
the compositor sizes it to the whole output, with `set_exclusive_zone(-1)` so it
neither reserves space nor pushes tiled windows around, and
`KeyboardInteractivity::None`.

Click-through is one call: `wl_surface.set_input_region` with an **empty**
region. A surface with no input region receives no pointer or touch events at
all, and the compositor routes them to whatever is underneath. Where the macOS
original had to fake this, Wayland makes it a first-class protocol feature.

That claim is cheap to make, so `--overlay-test` measures it. The overlay counts
every pointer event addressed to its own surface while a probe thread drives the
real cursor across it in six warps, over Hyprland's IPC:

```
layers:      on HDMI-A-2, level overlay, 1920x1080 at 0,0
cursor:      6/6 warps accepted, ended at 960,540
focus:       0x55f6ae230440  (unchanged)
pointer:     0 enters, 0 presses on the overlay surface
PASS: empty input region — the cursor crossed the whole surface and it never saw a thing.
```

A zero is only meaningful if a non-zero was possible, so `--overlay-test-control`
runs the identical sweep with the input region left alone:

```
pointer:     1 enters, 0 presses on the overlay surface
PASS (control): with an input region the same sweep IS seen, so the test can fail.
```

The test also aborts rather than passing if the cursor sweep does not complete —
a run where the pointer never moved would report zero events and prove nothing.
It is deliberately **not** in CI: it needs a live compositor, which no GitHub
runner has.

> Hyprland 0.56 moved dispatchers to Lua. The old flat `dispatch movecursor X Y`
> is now a syntax error rather than a no-op — the working form is
> `hl.dsp.cursor.move({x=X,y=Y})`, wrapped by `Hypr::move_cursor`. The first
> version of this test hit exactly that and reported a vacuous pass.

## The simulation

668 real FlyWire v783 neurons, 18,968 real signed synapses. The circuit is
LC4/LPLC2 looming detectors driving the DNp01 giant fibre, with DNa01/DNa02
steering, MDN backward walking, DNp09 forward walking, DNg11 grooming, and the
DNp02/04/11 escape-manoeuvre wing DNs — plus 330 synaptic partners so the
command neurons are not driven by noise alone.

(The 23,210-point figure sometimes quoted for this project is the *brain view's*
point cloud, `brain_points.json`. The simulated circuit is the 668.)

Every constant is upstream's: 20 ms membrane tau on a 1 ms step, threshold 1.0,
2 ms refractory, `weightScale` 0.0008, and the ×6 gap-junction boost on
LC→GF and wind→GF that documented electrical coupling needs because chemical
synapse counts under-represent it.

Two mechanisms carry most of the behaviour and are easy to break:

- **Escape is a race.** Excitation lands instantly; GABA and glutamate are
  queued 4 ms. The giant fibre fires in the gap before roughly 1,200 synapses of
  feedforward inhibition arrive. Slow ramps lose that race *by design* — escape
  is tested with abrupt loom steps, never ramps.
- **The operating point is razor-thin.** Neurons rest at `baseline × 20.4`
  against a threshold of 1.0, so scaling baselines linearly for a mood or
  time-of-day factor silences whole populations. The scale is compressed toward
  1 (`1 − (1−a)×0.35`) instead. `the_siesta_does_not_paralyse_the_fly` is the
  test that catches a regression here.

### Divergences from the original, and why

1. **Seeded RNG.** Upstream draws from `SystemRandomNumberGenerator`, so its
   runs cannot be replayed. `gnat` uses a seeded xoshiro128++, and the invariant
   suite asserts across four fixed seeds — an invariant that only holds for one
   lucky seed is not an invariant. Spike-for-spike equality with the Swift build
   is therefore impossible, which is fine: the invariants are statistical.
2. **No `.gnat` binary format.** An earlier draft invented one. `circuit.json`
   parses in milliseconds and staying byte-compatible with upstream's `etl.py`
   means a regenerated circuit drops in without touching Rust.
3. **No stim mutex.** The original locks because clicks arrive on the AppKit
   thread. Nothing here is threaded yet; the lock goes back in with the brain
   view, not before.

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
velocity between polls. The 4 ms escape latency lives inside the circuit, not in
the input rate, so it survives intact.

### Clicks

There is no honest way to get global button events as a normal Wayland client,
and Hyprland does not expose them over IPC. Rather than pretend, this sense is
downgraded: taps are derived from window open/close and focus changes. It is the
one macOS capability with no Wayland analog, and it is a security-model
decision, not an engineering gap.

## Milestones

1. ~~Desktop sense layer against live Hyprland.~~ **Done.**
2. ~~Port the simulation core and reproduce the documented invariants.~~ **Done.**
3. ~~Click-through layer-shell overlay, measured rather than asserted.~~ **Done.**
4. **Fly body and gait** (`FlyModel.swift`), plus the 17 `--behaviortest`
   checks, driven by the sim's `BrainSignals`.
5. Wire the senses to the sim: window ledges as terrain, cursor as loom.
6. Brain view — an ordinary xdg-toplevel, because it wants clicks.
7. Swap the software canvas for GL once the point cloud needs it.
8. Control surface: a Waybar module or a socket plus `gnat pause` / `gnat add`.

## Data

`data/circuit.json` and `data/brain_points.json` are vendored from upstream,
derived from FlyWire Codex FAFB v783. They are **CC BY-NC 4.0** — see
[`data/DATA_LICENSE.md`](data/DATA_LICENSE.md) for attribution and the papers to
cite. The code is MIT.

Regenerating them needs the raw Codex dumps and upstream's `etl.py`; there is no
reason to port that step, since its output is committed and stable.

## Credits

- **[DesktopFly](https://github.com/DenisSergeevitch/desktop-fly)** by Denis
  Shiryaev — the original, and the source of every simulation constant here.
- **[FlyWire](https://flywire.ai)** (Princeton and collaborators) for the
  connectome.
