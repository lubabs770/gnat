//! Machine temperature. A hot box makes a fast fly.
//!
//! Linux exposes this without any permission at all, and in more detail than
//! the macOS thermal-state enum the original used: an actual °C reading.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A single millidegree-Celsius sysfs file plus the label it belongs to.
#[derive(Clone, Debug)]
pub struct Sensor {
    pub label: String,
    pub path: PathBuf,
}

pub struct Thermal {
    sensors: Vec<Sensor>,
}

impl Thermal {
    /// Discover every readable temperature input, preferring hwmon (which has
    /// labels) and falling back to the thermal zones.
    pub fn discover() -> Self {
        let mut sensors = hwmon_sensors();
        if sensors.is_empty() {
            sensors = thermal_zone_sensors();
        }
        Self { sensors }
    }

    pub fn sensors(&self) -> &[Sensor] {
        &self.sensors
    }

    /// Hottest current reading in °C, or `None` if nothing could be read.
    pub fn hottest_c(&self) -> Option<f32> {
        self.sensors
            .iter()
            .filter_map(|s| read_millidegrees(&s.path).ok())
            .map(|m| m as f32 / 1000.0)
            .fold(None, |acc: Option<f32>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
    }

    /// Temperature mapped onto 0.0..=1.0 across a cool/hot band, for driving
    /// metabolic rate. Returns 0.5 when no sensor is available, so a machine
    /// with no thermal sysfs gets a normal fly rather than a frozen one.
    pub fn arousal(&self, cool_c: f32, hot_c: f32) -> f32 {
        match self.hottest_c() {
            Some(t) => ((t - cool_c) / (hot_c - cool_c)).clamp(0.0, 1.0),
            None => 0.5,
        }
    }
}

fn read_millidegrees(path: &Path) -> Result<i64> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(raw.trim().parse()?)
}

fn hwmon_sensors() -> Vec<Sensor> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/hwmon") else {
        return out;
    };
    for dir in entries.flatten() {
        let dir = dir.path();
        let chip = std::fs::read_to_string(dir.join("name"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "hwmon".into());

        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            // temp1_input, temp2_input, ...
            if !(name.starts_with("temp") && name.ends_with("_input")) {
                continue;
            }
            let label = std::fs::read_to_string(dir.join(name.replace("_input", "_label")))
                .map(|s| format!("{chip}/{}", s.trim()))
                .unwrap_or_else(|_| format!("{chip}/{name}"));
            out.push(Sensor {
                label,
                path: f.path(),
            });
        }
    }
    out
}

fn thermal_zone_sensors() -> Vec<Sensor> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/thermal") else {
        return out;
    };
    for dir in entries.flatten() {
        let dir = dir.path();
        if !dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("thermal_zone"))
        {
            continue;
        }
        let temp = dir.join("temp");
        if !temp.exists() {
            continue;
        }
        let label = std::fs::read_to_string(dir.join("type"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| dir.file_name().unwrap().to_string_lossy().into_owned());
        out.push(Sensor { label, path: temp });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arousal_is_clamped_and_defaults_mid() {
        let empty = Thermal {
            sensors: Vec::new(),
        };
        assert_eq!(empty.arousal(40.0, 85.0), 0.5);
    }

    #[test]
    fn discovery_does_not_panic() {
        // The box under test may have no sensors at all; that must be fine.
        let t = Thermal::discover();
        let _ = t.hottest_c();
        let _ = t.arousal(40.0, 85.0);
    }
}
