//! The circuit: real FlyWire v783 neurons and real signed synapse weights.
//!
//! Loaded straight from `data/circuit.json`, the artefact the upstream
//! `etl.py` produces from the FlyWire Codex dumps. This is deliberately not a
//! bespoke binary format — the JSON is 400 KB, parses in milliseconds, and
//! staying byte-compatible with upstream means a regenerated circuit drops in
//! without touching Rust.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// A neuron's part in the escape/steering circuit.
///
/// The first nine are the core populations `etl.py` picks out by primary cell
/// type; everything else is a synaptic partner pulled in to keep the command
/// neurons from being driven by noise alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Role {
    /// LC4 — looming detector, giant-fibre input.
    Lc4,
    /// LPLC2 — looming detector, giant-fibre input.
    Lplc2,
    /// DNp01, the giant fibre. The escape command neuron.
    Gf,
    /// DNa01 — steering.
    Dna01,
    /// DNa02 — steering.
    Dna02,
    /// MDN, the moonwalker: backward walking.
    Mdn,
    /// DNp09 — forward-walking command.
    Dnp09,
    /// DNg11 — grooming command.
    Dng11,
    /// DNp02/04/11 — loom-responsive escape-manoeuvre (wing) DNs.
    Escw,
    Other,
}

impl Role {
    fn parse(s: &str) -> Self {
        match s {
            "lc4" => Role::Lc4,
            "lplc2" => Role::Lplc2,
            "gf" => Role::Gf,
            "dna01" => Role::Dna01,
            "dna02" => Role::Dna02,
            "mdn" => Role::Mdn,
            "dnp09" => Role::Dnp09,
            "dng11" => Role::Dng11,
            "escw" => Role::Escw,
            _ => Role::Other,
        }
    }

    /// Whether this population drives the giant fibre through gap junctions,
    /// which chemical synapse counts under-represent.
    pub fn is_looming(self) -> bool {
        matches!(self, Role::Lc4 | Role::Lplc2)
    }

    /// Whether this is a steering descending neuron.
    pub fn is_steering(self) -> bool {
        matches!(self, Role::Dna01 | Role::Dna02)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
    Center,
}

impl Side {
    fn parse(s: &str) -> Self {
        match s {
            "left" => Side::Left,
            "right" => Side::Right,
            _ => Side::Center,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RawNeuron {
    id: String,
    #[serde(rename = "type")]
    cell_type: String,
    role: String,
    side: String,
    pos: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawCircuit {
    neurons: Vec<RawNeuron>,
    /// `[pre_index, post_index, signed_synapse_count]`.
    edges: Vec<[f32; 3]>,
}

#[derive(Clone, Debug)]
pub struct Neuron {
    /// FlyWire root id.
    pub id: String,
    /// Primary cell type for core members, super class for partners.
    pub cell_type: String,
    pub role: Role,
    pub side: Side,
    /// Normalised brain-space position, for the brain view.
    pub pos: [f32; 3],
}

pub struct Circuit {
    pub neurons: Vec<Neuron>,
    /// `(pre, post, signed synapse count)`, before any weight scaling.
    pub edges: Vec<(u32, u32, f32)>,
}

impl Circuit {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(json: &str) -> Result<Self> {
        let raw: RawCircuit = serde_json::from_str(json)?;
        let n = raw.neurons.len();

        let neurons = raw
            .neurons
            .into_iter()
            .map(|r| Neuron {
                id: r.id,
                role: Role::parse(&r.role),
                side: Side::parse(&r.side),
                pos: match r.pos.as_slice() {
                    [x, y, z] => [*x, *y, *z],
                    _ => [0.0, 0.0, 0.0],
                },
                cell_type: r.cell_type,
            })
            .collect();

        let mut edges = Vec::with_capacity(raw.edges.len());
        for [pre, post, w] in raw.edges {
            let (pre, post) = (pre as usize, post as usize);
            anyhow::ensure!(
                pre < n && post < n,
                "edge {pre}->{post} out of range for {n} neurons"
            );
            edges.push((pre as u32, post as u32, w));
        }

        Ok(Circuit { neurons, edges })
    }

    pub fn len(&self) -> usize {
        self.neurons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.neurons.is_empty()
    }

    /// Indices of every neuron with the given role.
    pub fn by_role(&self, role: Role) -> Vec<usize> {
        self.indices(|n| n.role == role)
    }

    /// Indices of every neuron matching a predicate.
    pub fn indices(&self, f: impl Fn(&Neuron) -> bool) -> Vec<usize> {
        self.neurons
            .iter()
            .enumerate()
            .filter(|(_, n)| f(n))
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "neurons": [
            {"id": "1", "type": "LC4",   "role": "lc4",   "side": "left",   "pos": [1, 2, 3]},
            {"id": "2", "type": "DNp01", "role": "gf",    "side": "center", "pos": [0, 0, 0]},
            {"id": "3", "type": "ascending", "role": "other", "side": "right", "pos": [4, 5, 6]}
        ],
        "edges": [[0, 1, 12.0], [2, 1, -8.0]],
        "source": "test"
    }"#;

    #[test]
    fn parses_the_upstream_shape() {
        let c = Circuit::parse(SAMPLE).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c.neurons[0].role, Role::Lc4);
        assert_eq!(c.neurons[0].side, Side::Left);
        assert_eq!(c.neurons[0].pos, [1.0, 2.0, 3.0]);
        assert_eq!(c.neurons[1].role, Role::Gf);
        assert_eq!(c.neurons[2].cell_type, "ascending");
        assert_eq!(c.edges, vec![(0, 1, 12.0), (2, 1, -8.0)]);
    }

    #[test]
    fn unknown_roles_become_other() {
        assert_eq!(Role::parse("dnp99"), Role::Other);
        assert_eq!(Side::parse("dorsal"), Side::Center);
    }

    #[test]
    fn rejects_an_out_of_range_edge() {
        let bad = SAMPLE.replace("[[0, 1, 12.0]", "[[0, 99, 12.0]");
        assert!(Circuit::parse(&bad).is_err());
    }

    #[test]
    fn selects_populations_by_role() {
        let c = Circuit::parse(SAMPLE).unwrap();
        assert_eq!(c.by_role(Role::Gf), vec![1]);
        assert_eq!(c.indices(|n| n.cell_type == "ascending"), vec![2]);
    }
}
