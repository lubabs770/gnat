# gnat

A fruit fly, simulated from its connectome, living on your Hyprland desktop.

A Linux/Wayland port of [**DesktopFly**](https://github.com/DenisSergeevitch/desktop-fly)
by Denis Shiryaev (MIT). The fly walks along the top edges of your windows,
startles when the cursor lunges at it, sleeps at night, and speeds up when the
laptop gets hot — and every one of those behaviours falls out of a
leaky integrate-and-fire simulation of a real *Drosophila* connectome, not a
state machine.

## Status

**It runs.** There is a fly on the screen, walking on your windows, startling
when the cursor lunges at it, driven by 668 real neurons — and a brain window
you can click to stimulate them. Both of the original's ground-truth suites
pass: the circuit invariants and all 17 end-to-end behaviour checks.

```
cargo build --release && ln -s "$PWD/target/release/gnat" ~/.local/bin/gnat
```

```
gnat                                         # put a fly on the screen
gnat --brain                                 # the same, plus the brain window
gnat --flies 8                               # start with eight of them
```

The connectome is found relative to the executable, so a symlink on `PATH`
works from any directory. `GNAT_DATA` overrides it, and an override that is
wrong is an error rather than a silent fallback — otherwise you can never be
sure which data you are looking at.

```
gnat pause | resume | toggle | scare | quit | status
gnat flies 8                            # set how many flies there are
gnat add | remove                       # nudge that by one
gnat brain                              # open the brain window on a running fly
gnat outputs                            # list outputs for --output
gnat waybar                             # one line of JSON for a bar module
```

```
cargo test --workspace                            # 91 tests
gnat --simtest                                    # circuit invariants, on real data
gnat --behaviortest                               # stimulate neurons, watch the body react
gnat --snapshot f.png                             # headless render, plus a zoomed crop
gnat --brainshot b.png                            # headless render of the brain view
gnat --senses                                     # one reading from every desktop sense
gnat --overlay-test                               # measure the click-through claim
cargo run -p gnat-senses --example probe          # live 20s dump of the whole sense layer
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
| `gnat-body` | Behaviour states, gait, flight, ledges, sleep, and the 17-check behaviour suite. No OS dependencies. | **ported** |
| `gnat-senses` | Hyprland IPC, window terrain, looming, thermal, clock, activity. | **working** |
| `gnat-overlay` | The click-through `wlr-layer-shell` surface, and a software canvas. | **working** |
| `gnat` | Coordinator, renderer, brain view, control socket, snapshot tools. | **running** |

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
4. **Two coordinate frames, named.** `gnat-body` works in the original's scene
   frame (origin at the centre of the output, +y up) so every ported constant
   keeps its meaning. `gnat-senses` reports screen coordinates, because that is
   what Hyprland reports. `coord.rs` converts, and is the only place that knows.
5. **Clicks become focus changes.** The original stimulates the sensory
   population on any global mouse click. A Wayland client cannot see those, so
   `activewindow` events stand in — a weaker signal, honestly weaker, rather
   than a pretend one.
6. **Temperature interpolates.** macOS gives four thermal buckets and the
   original steps tempo between them. `/sys/class/hwmon` gives °C, so this maps
   45-90 °C onto the same 1.0-1.5 tempo range continuously.

## The body

FlyWire has no body data, so unlike the circuit this part is *modelled* rather
than measured — the connectome decides what the fly does, `gnat-body` decides
what that looks like. Five states (walking, idle, grooming, flying, sleeping), a
tripod gait, ledge-following, a flight arc with a touchdown flare, and wings.

Every behavioural decision reads a real population's rate, through
`SignalBuilder`: DNp09 sets walking speed, DNg11 toggles grooming, MDN reverses
the fly, the DNa01/DNa02 left-right difference steers, DNp02/04/11 raises the
wings, and a giant-fibre spike takes off. The one piece of cleverness there is
worth knowing about:

> The DNa left-right difference is high-pass filtered with an 8 s time constant.
> The connectome has a *persistent* steering asymmetry, so feeding the raw
> difference to the body makes the fly walk in circles forever. Adapting the
> baseline out means steady-state walking is straight and only transient
> asymmetries — something seen, or a click — actually steer.

`--behaviortest` is the original's second ground-truth suite: seven scenarios
that stimulate real neurons and check what the body does, and ten that hand-build
signals to exercise body mechanics. All 17 pass.

```
PASS  GF stim -> escape flight: state=Flying
PASS  DNp09 stim -> walks, speed rises (capped): state=Walking speed=41
PASS  DNa-left stim -> left (CCW) turn while walking: heading change +0.27 rad
PASS  ledge attach + follow window edge: attached, y=-46
PASS  window closes underfoot -> takeoff: took off
PASS  sleep signal -> sleeping; wake -> grooming: woke to Grooming
PASS  thermal tempo scales walking speed: cool 46 -> hot 70 pt/s
PASS  landing is smooth: no scale/height snap at touchdown: landed=yes, max per-frame d-scale 0.06, d-z 5.5
...
ALL BEHAVIOR TESTS PASS (17)
```

Three of the seventeen are genuinely stochastic, because they drive a noisy
network and a fly that wanders. Measured over 40 seeds
(`cargo run --release -p gnat-body --example seed_survey`):

| check | pass rate |
|---|---|
| ledge attach + follow window edge | 85% |
| DNp09 stim -> walks, speed rises | 92% |
| DNa-left stim -> left turn | 98% |
| the other fourteen | 100% |

The original has exactly the same property and no way to see it — it draws a
fresh system-random seed every run, so a flaky check there just looks like an
occasional mystery failure. Rather than loosen the checks to hide it, the tests
assert what is actually true: the shipped seed is green, and every check clears
75% across a seed sweep. A real regression drops a check to near zero.

## Putting it together

One frame, in order: sense the world, drive the circuit, step it in whole
milliseconds, read the population rates as body commands, move the body, draw.

The cursor is polled every frame and turned into a looming stimulus — angular
expansion rate attenuated by distance, split between the two eyes by bearing
relative to the fly's heading, plus an air puff from raw cursor speed. That
transduction is the *only* modelling step on the input side; everything
downstream of the LC4/LPLC2 population is the real connectome. Window geometry
is re-read at 1.4 Hz, and a `closewindow` event forces an immediate refresh,
because the fly may be standing on what just vanished.

Two coordinate frames meet here and `coord.rs` is the only file allowed to know
about both: Hyprland reports screen coordinates (top-left origin, +y down) and
the body model works in the original's scene frame (centre origin, +y up). A
sign error in that conversion looks exactly like a behaviour bug, which is why
it lives in one named place with its own tests rather than being folded into
either crate.

### Drawing

The original is SceneKit and gets lighting and depth for free. This is a plain
pixel buffer, so altitude is conveyed the way an animator would: the fly scales
up, and its shadow slides away and softens.

`--snapshot` renders headlessly and writes both a full frame and a 6x crop
centred on the fly, composited over a checkerboard — the canvas is transparent
by design, so without that a correct render and an empty one look identical. It
also reports what the fly is standing on:

```
state    Idle
position 346,969 on screen  (alt 0.00)
terrain  4 ledges, standing on nothing
  ledge  y=  -540  x   -960..960     window 0x0
  ledge  y=   528  x      5..948     window 0x55f6ae230440
  ledge  y=   528  x   -948..-5      window 0x55f6ae27acb0
```

Two things that snapshot caught, which no test would have:

- The first render was unmistakably a **spider** — legs too long, splayed too
  wide, radiating from the body like spokes. Real fly legs tuck close underneath.
- Every body segment was drawn **broadside-on**. `Canvas::ellipse` puts `rx`
  along the angle it is given and `ry` across it, so passing the heading directly
  rotates the fly a quarter turn. Not subtle once you look: it turns the fly into
  a blob.

## The brain view

`--brain` opens a second window — an ordinary xdg-toplevel, not a layer surface,
because unlike the fly it very much wants to be clicked. It draws all 23,210
somas coloured by super class, the 668 simulated neurons brighter on top
coloured by role, and a flash wherever something spikes. Click anywhere to
stimulate the nearest cluster; it names what you hit.

It is software 3D. The point cloud is accumulated additively into a float buffer
and tone-mapped at the end, which needs no depth sorting — order-independent by
construction, and it gives a point cloud the glow it wants anyway.

The view runs on **its own thread and its own Wayland connection**, so a slow
repaint there cannot stall the fly. It talks to the sim through two small
channels: spikes out on a bounded `SpikeBus`, stimulation back in on a
`StimQueue`. That is the original's arrangement too — its brain panel renders on
the AppKit thread while the sim advances in the fly's render loop — and it is
where the stim mutex I deferred in milestone 2 finally earns its keep.

`--brainshot` renders it headlessly, driving a real loom so the shot has an
actual giant-fibre volley in it rather than baseline crackle.

There is no font on a raw pixel buffer, so `font.rs` is a 3x5 bitmap alphabet.
Uppercase only: three pixels wide is the narrowest a letter can be and stay
readable, and the labels are short.

## More than one fly

`gnat flies N` sets how many there are — usually what you actually want, since
the answer is "four", not "three more than whatever is there now". `--flies N`
does the same at startup, and `gnat add` / `gnat remove` nudge by one. `gnat
flies` with no number just reports the count. The ceiling is 64, so a typo
cannot spawn a thousand.

Only the **first** fly is wired to the connectome — the rest run the original's
legacy distance-based fear, which is what it does too. One brain is plenty, and
six would be six times the work for no extra behaviour. `remove` will not take
the first one, because it is the one carrying the brain.

`gnat scare` reflects that split honestly: it raises the looming drive for the
fly that has a connectome and lets it decide, and simply launches the others.

## The control surface

The original hangs this off a menu-bar item. Wayland has no global menu bar, and
a socket plus a CLI fits a tiling desktop better:

```
$ gnat status
{"state":"flying","paused":false,"sleeping":false,"pop_hz":7.26,"neurons":668,"ledges":5}

$ gnat scare      # a real 0.6 loom into the real circuit, then it decays
ok

$ gnat pause
ok paused
```

`scare` is deliberately not a scripted takeoff. It raises the looming drive and
lets the connectome decide, exactly like the original's "scare all" — so it can
fail to produce an escape, which is the point.

For Waybar:

```jsonc
"custom/gnat": {
    "exec": "gnat waybar",
    "return-type": "json",
    "interval": 2,
    "on-click": "gnat toggle",
    "on-click-right": "gnat scare"
}
```

The module degrades to an empty label when no fly is running, rather than
filling the bar with errors, and shows a count once there is more than one fly:
`grooming x3`.

Ready-made copies of both live in [`packaging/`](packaging), along with a
systemd user unit:

```
cp packaging/gnat.service ~/.config/systemd/user/
systemctl --user enable --now gnat
```

## Outputs

`gnat outputs` lists them; `gnat --output HDMI-A-2` pins the overlay to one.

Output *names* arrive as Wayland events rather than as registry globals, so they
cannot be matched until the registry has been pumped — the first version of
`--output` never matched anything for exactly that reason. Resolution now
happens on a throwaway event queue before the real surface is built, and an
unknown name says what it did find:

```
$ gnat --output NOPE-1
Error: no output named NOPE-1; this compositor reports ["HDMI-A-2"]
```

Spanning *all* outputs at once — one layer surface per monitor, the fly walking
between screens — is **not** done. It needs multi-surface support on one
connection, and this machine has a single monitor, so it would be untested code.
Written down rather than guessed at.

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
| Menu bar item | `NSStatusItem` | Control socket + CLI + Waybar module | Reworked, and scriptable. |

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
4. ~~Fly body and gait, and all 17 `--behaviortest` checks.~~ **Done.**
5. ~~Wire it together and draw the fly.~~ **Done.**
6. ~~Brain view: 23,210 point sprites, click-to-stimulate.~~ **Done.**
7. ~~Control surface: a socket, a CLI, and a Waybar module.~~ **Done.**
8. ~~Extra flies, output selection, a runtime brain toggle, packaging.~~ **Done.**
9. **Multi-monitor spanning** — one layer surface per output. Needs multi-surface
   support on a single connection, and a second monitor to test against.
10. Swap the software canvas for GL if the point cloud ever needs it. It does
    not yet: 24k points is nothing.
11. The original's remaining extras — `--behaviortest` has a sibling suite of
    render snapshots upstream that this port does not have.

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
