//! Hyprland IPC.
//!
//! Two sockets live under `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`:
//!
//! * `.socket.sock`  — request/response. One command per connection.
//! * `.socket2.sock` — a line-oriented event stream, held open.
//!
//! We talk to both directly rather than shelling out to `hyprctl`, because the
//! looming pathway wants the cursor at display rate and spawning a process
//! sixty times a second is not acceptable.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn socket_dir() -> Result<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .context("HYPRLAND_INSTANCE_SIGNATURE is not set; not running under Hyprland?")?;
    Ok(PathBuf::from(runtime).join("hypr").join(sig))
}

/// A window, as the fly sees it: a rectangle whose top edge is a ledge.
#[derive(Clone, Debug, Deserialize)]
pub struct Client {
    pub address: String,
    pub at: (i32, i32),
    pub size: (i32, i32),
    pub class: String,
    pub title: String,
    pub floating: bool,
    pub mapped: bool,
    pub hidden: bool,
    pub monitor: i32,
    /// 0 is the focused window, 1 the previously focused, and so on. Combined
    /// with `floating` this is the closest thing Hyprland's IPC gives us to a
    /// stacking order.
    #[serde(rename = "focusHistoryID")]
    pub focus_history_id: i32,
}

impl Client {
    pub fn left(&self) -> i32 {
        self.at.0
    }
    pub fn top(&self) -> i32 {
        self.at.1
    }
    pub fn right(&self) -> i32 {
        self.at.0 + self.size.0
    }
    pub fn bottom(&self) -> i32 {
        self.at.1 + self.size.1
    }
    /// The window address as a number, for use as a stable ledge identity.
    /// Hyprland reports it as `0x55f6ae1c4d30`.
    pub fn id(&self) -> u64 {
        u64::from_str_radix(self.address.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    pub fn area(&self) -> i64 {
        self.size.0 as i64 * self.size.1 as i64
    }
    /// Whether the fly can stand on this window at all.
    pub fn is_terrain(&self) -> bool {
        self.mapped && !self.hidden && self.size.0 > 0 && self.size.1 > 0
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left() && x < self.right() && y >= self.top() && y < self.bottom()
    }
}

/// Sort a client list front-to-back.
///
/// `clients` comes back in creation order, which is not what is on top. Two
/// rules recover an approximate stacking order: floating windows always sit
/// above tiled ones, and within each group the more recently focused window is
/// in front.
pub fn sort_front_to_back(clients: &mut [Client]) {
    clients.sort_by_key(|c| (!c.floating, c.focus_history_id));
}

/// One layer-shell surface, as reported by `j/layers`.
#[derive(Clone, Debug, Deserialize)]
pub struct LayerSurface {
    pub namespace: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Which wlr-layer-shell layer a surface sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerLevel {
    Background,
    Bottom,
    Top,
    Overlay,
}

impl LayerLevel {
    pub fn from_index(i: u8) -> Option<Self> {
        Some(match i {
            0 => LayerLevel::Background,
            1 => LayerLevel::Bottom,
            2 => LayerLevel::Top,
            3 => LayerLevel::Overlay,
            _ => return None,
        })
    }
}

impl std::fmt::Display for LayerLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LayerLevel::Background => "background",
            LayerLevel::Bottom => "bottom",
            LayerLevel::Top => "top",
            LayerLevel::Overlay => "overlay",
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct CursorPos {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Monitor {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    #[serde(rename = "refreshRate")]
    pub refresh_rate: f32,
}

/// Request half of the IPC. Cheap to construct; each call opens its own
/// connection, which is what Hyprland expects.
pub struct Hypr {
    dir: PathBuf,
}

impl Hypr {
    pub fn connect() -> Result<Self> {
        let dir = socket_dir()?;
        let sock = dir.join(".socket.sock");
        if !sock.exists() {
            bail!("{} does not exist", sock.display());
        }
        Ok(Self { dir })
    }

    /// Send one command and return the raw reply.
    pub fn request(&self, command: &str) -> Result<String> {
        let mut stream = UnixStream::connect(self.dir.join(".socket.sock"))
            .context("connecting to the Hyprland request socket")?;
        stream.write_all(command.as_bytes())?;
        stream.flush()?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply)?;
        Ok(reply)
    }

    fn request_json<T: for<'de> Deserialize<'de>>(&self, command: &str) -> Result<T> {
        let raw = self.request(&format!("j/{command}"))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("parsing the reply to `{command}`: {raw:.200}"))
    }

    /// Every window Hyprland knows about, including ones on other workspaces,
    /// already sorted front-to-back by [`sort_front_to_back`].
    pub fn clients(&self) -> Result<Vec<Client>> {
        let mut clients: Vec<Client> = self.request_json("clients")?;
        sort_front_to_back(&mut clients);
        Ok(clients)
    }

    /// Global cursor position in layout coordinates.
    pub fn cursor_pos(&self) -> Result<CursorPos> {
        self.request_json("cursorpos")
    }

    pub fn monitors(&self) -> Result<Vec<Monitor>> {
        self.request_json("monitors")
    }

    /// The focused window, or `None` when nothing is focused.
    pub fn active_window(&self) -> Result<Option<Client>> {
        let raw = self.request("j/activewindow")?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing activewindow: {raw:.200}"))?;
        // An unfocused desktop answers with `{}` rather than null.
        if value.as_object().is_some_and(|o| o.is_empty()) {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(value)?))
    }

    /// Every layer-shell surface, as `(monitor, level, surface)`.
    pub fn layers(&self) -> Result<Vec<(String, LayerLevel, LayerSurface)>> {
        #[derive(Deserialize)]
        struct MonitorLayers {
            levels: std::collections::HashMap<String, Vec<LayerSurface>>,
        }
        let by_monitor: std::collections::HashMap<String, MonitorLayers> =
            self.request_json("layers")?;

        let mut out = Vec::new();
        for (monitor, m) in by_monitor {
            for (level, surfaces) in m.levels {
                let Some(level) = level.parse().ok().and_then(LayerLevel::from_index) else {
                    continue;
                };
                for s in surfaces {
                    out.push((monitor.clone(), level, s));
                }
            }
        }
        Ok(out)
    }

    /// Run a Hyprland dispatcher.
    ///
    /// Hyprland 0.56 replaced the flat `dispatch movecursor 400 400` syntax
    /// with Lua: the server wraps whatever it receives in
    /// `return hl.dispatch(...)`, so the argument has to be a dispatcher
    /// expression and the old form is a syntax error, not a no-op.
    pub fn dispatch(&self, lua: &str) -> Result<()> {
        let reply = self.request(&format!("dispatch {lua}"))?;
        anyhow::ensure!(reply.trim() == "ok", "dispatch `{lua}` said: {reply}");
        Ok(())
    }

    /// Warp the cursor to absolute layout coordinates.
    pub fn move_cursor(&self, x: i32, y: i32) -> Result<()> {
        self.dispatch(&format!("hl.dsp.cursor.move({{x={x},y={y}}})"))
    }
}

/// One line off the event socket, already split into a kind and its payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    OpenWindow {
        address: String,
        workspace: String,
        class: String,
        title: String,
    },
    CloseWindow {
        address: String,
    },
    MoveWindow {
        address: String,
        workspace: String,
    },
    ActiveWindow {
        class: String,
        title: String,
    },
    Workspace {
        name: String,
    },
    MonitorAdded {
        name: String,
    },
    MonitorRemoved {
        name: String,
    },
    ChangeFloatingMode {
        address: String,
        floating: bool,
    },
    Fullscreen {
        entered: bool,
    },
    /// Anything we do not model yet, kept so the caller can log it.
    Other {
        kind: String,
        payload: String,
    },
}

impl Event {
    fn parse(line: &str) -> Option<Self> {
        let (kind, payload) = line.split_once(">>")?;
        // Payload fields are comma-separated, and the last field (a title) may
        // itself contain commas, so every split here is bounded.
        let f: Vec<&str> = payload.splitn(4, ',').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or_default().to_string();

        Some(match kind {
            "openwindow" => Event::OpenWindow {
                address: get(0),
                workspace: get(1),
                class: get(2),
                title: get(3),
            },
            "closewindow" => Event::CloseWindow { address: get(0) },
            "movewindow" => Event::MoveWindow {
                address: get(0),
                workspace: get(1),
            },
            "activewindow" => Event::ActiveWindow {
                class: get(0),
                title: get(1),
            },
            "workspace" => Event::Workspace { name: get(0) },
            "monitoradded" => Event::MonitorAdded { name: get(0) },
            "monitorremoved" => Event::MonitorRemoved { name: get(0) },
            "changefloatingmode" => Event::ChangeFloatingMode {
                address: get(0),
                floating: get(1) == "1",
            },
            "fullscreen" => Event::Fullscreen {
                entered: get(0) == "1",
            },
            other => Event::Other {
                kind: other.to_string(),
                payload: payload.to_string(),
            },
        })
    }
}

/// Blocking iterator over the event socket. Intended to be driven on its own
/// thread, feeding a channel.
pub struct EventStream {
    lines: std::io::Lines<BufReader<UnixStream>>,
}

impl EventStream {
    pub fn connect() -> Result<Self> {
        let path = socket_dir()?.join(".socket2.sock");
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        Ok(Self {
            lines: BufReader::new(stream).lines(),
        })
    }
}

impl Iterator for EventStream {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        loop {
            let line = self.lines.next()?.ok()?;
            if let Some(event) = Event::parse(&line) {
                return Some(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events() {
        assert_eq!(
            Event::parse("closewindow>>55f6ae1c4d30"),
            Some(Event::CloseWindow {
                address: "55f6ae1c4d30".into()
            })
        );
        assert_eq!(
            Event::parse("changefloatingmode>>55f6ae1c4d30,1"),
            Some(Event::ChangeFloatingMode {
                address: "55f6ae1c4d30".into(),
                floating: true
            })
        );
    }

    #[test]
    fn keeps_commas_inside_a_window_title() {
        let e = Event::parse("openwindow>>abc,2,chromium,ENG-2489: enable IAP, widen WIF");
        assert_eq!(
            e,
            Some(Event::OpenWindow {
                address: "abc".into(),
                workspace: "2".into(),
                class: "chromium".into(),
                title: "ENG-2489: enable IAP, widen WIF".into(),
            })
        );
    }

    #[test]
    fn unknown_events_survive() {
        let e = Event::parse("somethingnew>>a,b").unwrap();
        assert!(matches!(e, Event::Other { .. }));
    }

    fn client(floating: bool, focus: i32) -> Client {
        Client {
            address: format!("{floating}{focus}"),
            at: (0, 0),
            size: (10, 10),
            class: "c".into(),
            title: "t".into(),
            floating,
            mapped: true,
            hidden: false,
            monitor: 0,
            focus_history_id: focus,
        }
    }

    #[test]
    fn floating_windows_sort_in_front_of_tiled_ones() {
        // A tiled window focused right now is still *behind* a stale floating one.
        let mut cs = vec![
            client(false, 0),
            client(true, 3),
            client(false, 1),
            client(true, 1),
        ];
        sort_front_to_back(&mut cs);
        let order: Vec<(bool, i32)> = cs
            .iter()
            .map(|c| (c.floating, c.focus_history_id))
            .collect();
        assert_eq!(order, vec![(true, 1), (true, 3), (false, 0), (false, 1)]);
    }

    #[test]
    fn window_addresses_parse_into_ids() {
        let mut c = client(false, 0);
        c.address = "0x55f6ae1c4d30".into();
        assert_eq!(c.id(), 0x55f6ae1c4d30);
        c.address = "not-hex".into();
        assert_eq!(c.id(), 0, "an unparseable address must not panic");
    }

    #[test]
    fn layer_levels_map_to_names() {
        assert_eq!(LayerLevel::from_index(3), Some(LayerLevel::Overlay));
        assert_eq!(LayerLevel::from_index(0), Some(LayerLevel::Background));
        assert_eq!(LayerLevel::from_index(9), None);
        assert_eq!(LayerLevel::Overlay.to_string(), "overlay");
    }

    #[test]
    fn client_geometry() {
        let c = Client {
            address: "x".into(),
            at: (12, 12),
            size: (1896, 1056),
            class: "chromium".into(),
            title: "t".into(),
            floating: false,
            mapped: true,
            hidden: false,
            monitor: 0,
            focus_history_id: 0,
        };
        assert_eq!(
            (c.left(), c.top(), c.right(), c.bottom()),
            (12, 12, 1908, 1068)
        );
        assert!(c.is_terrain());
        assert!(c.contains(100, 100));
        assert!(!c.contains(100, 2000));
    }
}
