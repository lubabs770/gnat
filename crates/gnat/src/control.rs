//! The control surface: a Unix socket, a handful of subcommands, and a Waybar
//! module.
//!
//! The original hangs this off a menu-bar item. Wayland has no global menu bar,
//! and a socket plus a CLI fits a tiling desktop better anyway — `gnat pause`
//! from any terminal, or a Waybar module polling `gnat waybar`.
//!
//! The protocol is one line in, one line out. It is not a network service: the
//! socket lives in `$XDG_RUNTIME_DIR`, which is already user-private.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What the running fly publishes, and what commands set.
#[derive(Clone, Debug)]
pub struct Shared {
    pub paused: bool,
    /// Set by `scare`, consumed by the next frame.
    pub scare: bool,
    pub quit: bool,
    pub state: String,
    /// Whole-population firing rate, Hz per neuron.
    pub pop_hz: f32,
    pub neurons: usize,
    pub ledges: usize,
    pub sleeping: bool,
    pub flies: usize,
    pub brain: bool,
    /// A request to open the brain view, consumed by the next frame.
    pub open_brain: bool,
    /// Flies asked for and not yet created, and the same in reverse.
    pub add_flies: u32,
    pub remove_flies: u32,
    /// An absolute count to reconcile to, from `gnat flies N`.
    pub target_flies: Option<usize>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            paused: false,
            scare: false,
            quit: false,
            state: "starting".into(),
            pop_hz: 0.0,
            neurons: 0,
            ledges: 0,
            sleeping: false,
            flies: 1,
            brain: false,
            open_brain: false,
            add_flies: 0,
            remove_flies: 0,
            target_flies: None,
        }
    }
}

pub type Control = Arc<Mutex<Shared>>;

/// An upper bound on flies. Each one is cheap, but a typo should not spawn a
/// thousand of them.
pub const MAX_FLIES: usize = 64;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("gnat.sock")
}

/// Start the control socket, replacing any stale one left by a crash.
///
/// Returns without a listener rather than failing the whole program: a fly you
/// cannot pause is better than no fly.
pub fn serve(control: Control) {
    let path = socket_path();
    // A leftover socket file is not a running server; connecting tells us which.
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("control socket unavailable at {}: {e}", path.display());
            return;
        }
    };

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = handle(stream, &control);
        }
    });
}

fn handle(stream: UnixStream, control: &Control) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let reply = {
        let mut c = control.lock().unwrap();
        match line.trim() {
            "pause" => {
                c.paused = true;
                "ok paused".to_string()
            }
            "resume" => {
                c.paused = false;
                "ok running".to_string()
            }
            "toggle" => {
                c.paused = !c.paused;
                if c.paused { "ok paused" } else { "ok running" }.to_string()
            }
            "scare" => {
                c.scare = true;
                "ok".to_string()
            }
            "brain" => {
                if c.brain {
                    "ok already open".to_string()
                } else {
                    c.open_brain = true;
                    "ok".to_string()
                }
            }
            // `flies N` sets an absolute count, which is what you want when
            // the answer is "four", not "three more than whatever is there".
            cmd if cmd.starts_with("flies") => match cmd.split_whitespace().nth(1) {
                Some(n) => match n.parse::<usize>() {
                    Ok(n) if (1..=MAX_FLIES).contains(&n) => {
                        c.target_flies = Some(n);
                        format!("ok {n}")
                    }
                    Ok(n) => format!("error {n} is outside 1..={MAX_FLIES}"),
                    Err(_) => format!("error {n:?} is not a number"),
                },
                None => format!("ok {}", c.flies),
            },
            "add" => {
                c.add_flies += 1;
                "ok".to_string()
            }
            "remove" => {
                c.remove_flies += 1;
                "ok".to_string()
            }
            "quit" => {
                c.quit = true;
                "ok".to_string()
            }
            "status" => status_json(&c),
            other => format!("error unknown command {other:?}"),
        }
    };

    let mut stream = stream;
    writeln!(stream, "{reply}")?;
    Ok(())
}

fn status_json(s: &Shared) -> String {
    format!(
        r#"{{"state":"{}","paused":{},"sleeping":{},"pop_hz":{:.2},"neurons":{},"ledges":{},"flies":{},"brain":{}}}"#,
        s.state, s.paused, s.sleeping, s.pop_hz, s.neurons, s.ledges, s.flies, s.brain
    )
}

/// Send one command to a running fly and return its reply.
pub fn send(command: &str) -> Result<String> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "no fly is running (nothing listening on {})",
            path.display()
        )
    })?;
    writeln!(stream, "{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

/// One line of Waybar JSON. Falls back to a dimmed module rather than an error
/// when no fly is running, so the bar does not fill with noise.
pub fn waybar() -> String {
    let Ok(raw) = send("status") else {
        return r#"{"text":"","tooltip":"gnat: not running","class":"off"}"#.to_string();
    };
    let get = |key: &str| -> Option<String> {
        let at = raw.find(&format!("\"{key}\":"))? + key.len() + 3;
        let rest = &raw[at..];
        let end = rest.find([',', '}'])?;
        Some(rest[..end].trim_matches('"').to_string())
    };

    let state = get("state").unwrap_or_else(|| "?".into());
    let flies: usize = get("flies").and_then(|f| f.parse().ok()).unwrap_or(1);
    let paused = get("paused").as_deref() == Some("true");
    let sleeping = get("sleeping").as_deref() == Some("true");
    let pop = get("pop_hz").unwrap_or_else(|| "0".into());

    let (icon, class) = if paused {
        ("paused", "paused")
    } else if sleeping {
        ("asleep", "sleeping")
    } else {
        (state.as_str(), "running")
    };
    // Only show a count when there is more than one, so the common case stays
    // a single short word.
    let count = if flies > 1 {
        format!(" x{flies}")
    } else {
        String::new()
    };
    format!(
        r#"{{"text":"{icon}{count}","tooltip":"gnat: {state}, {pop} Hz/neuron, {flies} flies","class":"{class}"}}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_valid_json_shaped_output() {
        let s = Shared {
            state: "walking".into(),
            pop_hz: 6.25,
            neurons: 668,
            ledges: 4,
            ..Shared::default()
        };
        let json = status_json(&s);
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(json.contains(r#""state":"walking""#), "{json}");
        assert!(json.contains(r#""paused":false"#), "{json}");
        assert!(json.contains(r#""pop_hz":6.25"#), "{json}");
        assert!(json.contains(r#""neurons":668"#), "{json}");
    }

    #[test]
    fn the_socket_lives_under_the_runtime_dir() {
        let p = socket_path();
        assert!(p.ends_with("gnat.sock"), "{}", p.display());
        assert!(p.is_absolute());
    }

    #[test]
    fn waybar_degrades_quietly_when_nothing_is_running() {
        // The real socket may or may not exist on this machine; the no-fly
        // branch is the one that must never look like an error.
        let out = waybar();
        assert!(out.starts_with('{') && out.ends_with('}'), "{out}");
        assert!(out.contains("\"class\""), "{out}");
    }

    /// Drive the real socket end to end on a private path.
    #[test]
    fn commands_round_trip_over_the_socket() {
        let dir = std::env::temp_dir().join(format!("gnat-ctl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY-ish: this test owns the variable for the process, and the
        // other tests here do not depend on its value.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };

        let control: Control = Arc::new(Mutex::new(Shared {
            state: "idle".into(),
            neurons: 668,
            ..Shared::default()
        }));
        serve(control.clone());
        // Give the listener a moment to come up.
        std::thread::sleep(std::time::Duration::from_millis(60));

        assert_eq!(send("pause").unwrap(), "ok paused");
        assert!(control.lock().unwrap().paused);
        assert_eq!(send("toggle").unwrap(), "ok running");
        assert!(!control.lock().unwrap().paused);
        assert_eq!(send("scare").unwrap(), "ok");
        assert!(control.lock().unwrap().scare);
        assert_eq!(send("flies 4").unwrap(), "ok 4");
        assert_eq!(control.lock().unwrap().target_flies, Some(4));
        assert!(send("flies 0").unwrap().starts_with("error"));
        assert!(send("flies 9999").unwrap().starts_with("error"));
        assert!(send("flies wasp").unwrap().starts_with("error"));
        // A bare `flies` reports rather than sets.
        assert_eq!(send("flies").unwrap(), "ok 1");

        assert_eq!(send("brain").unwrap(), "ok");
        assert!(control.lock().unwrap().open_brain);
        assert_eq!(send("add").unwrap(), "ok");
        assert_eq!(send("add").unwrap(), "ok");
        assert_eq!(send("remove").unwrap(), "ok");
        {
            let c = control.lock().unwrap();
            assert_eq!(
                (c.add_flies, c.remove_flies),
                (2, 1),
                "requests should queue"
            );
        }

        let status = send("status").unwrap();
        assert!(status.contains(r#""neurons":668"#), "{status}");
        assert!(send("nonsense").unwrap().starts_with("error unknown"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
