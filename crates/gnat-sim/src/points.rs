//! The brain's point cloud: every soma FlyWire has a coordinate for.
//!
//! Purely for looking at. Nothing here is simulated — the 668 neurons that are
//! live in [`crate::Lif`] are a tiny subset drawn on top of this.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct RawPoints {
    classes: Vec<String>,
    /// `[x, y, z, class_index]`.
    points: Vec<Vec<f32>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub pos: [f32; 3],
    /// Index into [`BrainPoints::classes`].
    pub class: u8,
}

pub struct BrainPoints {
    pub classes: Vec<String>,
    pub points: Vec<Point>,
}

impl BrainPoints {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawPoints = serde_json::from_str(json)?;
        let points = raw
            .points
            .iter()
            .filter(|p| p.len() >= 4)
            .map(|p| Point {
                pos: [p[0], p[1], p[2]],
                // Anything out of range falls back to "central", the way the
                // original's colour lookup does.
                class: if (p[3] as usize) < raw.classes.len() {
                    p[3] as u8
                } else {
                    1
                },
            })
            .collect();
        Ok(BrainPoints {
            classes: raw.classes,
            points,
        })
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "classes": ["optic", "central"],
        "points": [[1, 2, 3, 0], [4, 5, 6, 1], [7, 8, 9, 99], [1, 2]],
        "source": "test"
    }"#;

    #[test]
    fn parses_and_drops_short_rows() {
        let b = BrainPoints::parse(SAMPLE).unwrap();
        assert_eq!(b.len(), 3, "the two-element row should be dropped");
        assert_eq!(b.points[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(b.points[1].class, 1);
    }

    #[test]
    fn an_out_of_range_class_falls_back_rather_than_indexing_off_the_end() {
        let b = BrainPoints::parse(SAMPLE).unwrap();
        assert_eq!(b.points[2].class, 1);
    }
}
