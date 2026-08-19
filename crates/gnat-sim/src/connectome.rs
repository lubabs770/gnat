//! Connectome storage and the on-disk `.gnat` format.
//!
//! Edges are held in CSR (compressed sparse row) order keyed by presynaptic
//! neuron, which is the access pattern the spike loop wants: when neuron `i`
//! fires we need exactly the slice of its outgoing synapses.

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::Path;

/// `b"GNAT"` followed by the format version.
const MAGIC: [u8; 4] = *b"GNAT";
const VERSION: u32 = 1;

/// A neuron's coarse role, used by the sensory/motor bindings and by the
/// renderer to colour the brain view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Role {
    Interneuron = 0,
    Sensory = 1,
    Motor = 2,
}

impl Role {
    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Role::Interneuron,
            1 => Role::Sensory,
            2 => Role::Motor,
            other => bail!("unknown neuron role byte {other}"),
        })
    }
}

/// Per-neuron metadata. Kept parallel to the state arrays so the hot loop only
/// ever touches flat `Vec`s.
#[derive(Clone, Debug)]
pub struct Neuron {
    /// Stable upstream identifier (FlyWire root id, or whatever the ETL keyed on).
    pub root_id: u64,
    pub role: Role,
    /// Index into [`Connectome::type_names`].
    pub cell_type: u32,
    /// Position in brain space, used only for rendering.
    pub pos: [f32; 3],
}

pub struct Connectome {
    pub neurons: Vec<Neuron>,
    /// `offsets[i]..offsets[i + 1]` bounds neuron `i`'s outgoing edges.
    /// Length is `neurons.len() + 1`.
    pub offsets: Vec<u32>,
    pub targets: Vec<u32>,
    /// Signed synaptic weight: the neurotransmitter sign already folded into
    /// the synapse count, so the spike loop never branches on transmitter type.
    pub weights: Vec<f32>,
    pub type_names: Vec<String>,
}

impl Connectome {
    pub fn neuron_count(&self) -> usize {
        self.neurons.len()
    }

    pub fn synapse_count(&self) -> usize {
        self.targets.len()
    }

    /// Outgoing edges of `neuron` as `(target, weight)` slices.
    #[inline]
    pub fn out_edges(&self, neuron: usize) -> (&[u32], &[f32]) {
        let lo = self.offsets[neuron] as usize;
        let hi = self.offsets[neuron + 1] as usize;
        (&self.targets[lo..hi], &self.weights[lo..hi])
    }

    /// Indices of every neuron whose cell type name matches `name`.
    pub fn by_type(&self, name: &str) -> Vec<u32> {
        let Some(ty) = self.type_names.iter().position(|t| t == name) else {
            return Vec::new();
        };
        let ty = ty as u32;
        self.neurons
            .iter()
            .enumerate()
            .filter(|(_, n)| n.cell_type == ty)
            .map(|(i, _)| i as u32)
            .collect()
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut f = std::io::BufReader::new(
            std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?,
        );
        Self::read_from(&mut f).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn read_from(r: &mut impl Read) -> Result<Self> {
        let mut magic = [0u8; 4];
        r.read_exact(&mut magic)?;
        if magic != MAGIC {
            bail!("not a .gnat file (bad magic)");
        }
        if read_u32(r)? != VERSION {
            bail!("unsupported .gnat version");
        }

        let n_neurons = read_u32(r)? as usize;
        let n_edges = read_u32(r)? as usize;
        let n_types = read_u32(r)? as usize;

        let mut type_names = Vec::with_capacity(n_types);
        for _ in 0..n_types {
            let len = read_u32(r)? as usize;
            let mut buf = vec![0u8; len];
            r.read_exact(&mut buf)?;
            type_names.push(String::from_utf8(buf).context("cell type name is not UTF-8")?);
        }

        let mut neurons = Vec::with_capacity(n_neurons);
        for _ in 0..n_neurons {
            let root_id = read_u64(r)?;
            let role = Role::from_u8(read_u8(r)?)?;
            let cell_type = read_u32(r)?;
            let pos = [read_f32(r)?, read_f32(r)?, read_f32(r)?];
            neurons.push(Neuron {
                root_id,
                role,
                cell_type,
                pos,
            });
        }

        let offsets = read_u32_vec(r, n_neurons + 1)?;
        let targets = read_u32_vec(r, n_edges)?;
        let weights = read_f32_vec(r, n_edges)?;

        let c = Connectome {
            neurons,
            offsets,
            targets,
            weights,
            type_names,
        };
        c.validate()?;
        Ok(c)
    }

    pub fn write_to(&self, w: &mut impl Write) -> Result<()> {
        self.validate()?;
        w.write_all(&MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        w.write_all(&(self.neurons.len() as u32).to_le_bytes())?;
        w.write_all(&(self.targets.len() as u32).to_le_bytes())?;
        w.write_all(&(self.type_names.len() as u32).to_le_bytes())?;
        for name in &self.type_names {
            w.write_all(&(name.len() as u32).to_le_bytes())?;
            w.write_all(name.as_bytes())?;
        }
        for n in &self.neurons {
            w.write_all(&n.root_id.to_le_bytes())?;
            w.write_all(&[n.role as u8])?;
            w.write_all(&n.cell_type.to_le_bytes())?;
            for c in n.pos {
                w.write_all(&c.to_le_bytes())?;
            }
        }
        for v in &self.offsets {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &self.targets {
            w.write_all(&v.to_le_bytes())?;
        }
        for v in &self.weights {
            w.write_all(&v.to_le_bytes())?;
        }
        Ok(())
    }

    /// Structural checks that must hold before the spike loop can trust the
    /// CSR arrays and skip bounds checks on the hot path.
    pub fn validate(&self) -> Result<()> {
        let n = self.neurons.len();
        if self.offsets.len() != n + 1 {
            bail!(
                "offsets length {} != neurons + 1 ({})",
                self.offsets.len(),
                n + 1
            );
        }
        if self.targets.len() != self.weights.len() {
            bail!("targets/weights length mismatch");
        }
        if *self.offsets.last().unwrap_or(&0) as usize != self.targets.len() {
            bail!("final offset does not equal edge count");
        }
        if self.offsets.windows(2).any(|w| w[0] > w[1]) {
            bail!("offsets are not monotonically non-decreasing");
        }
        if let Some(bad) = self.targets.iter().find(|&&t| t as usize >= n) {
            bail!("edge targets neuron {bad}, out of range for {n} neurons");
        }
        if let Some(bad) = self
            .neurons
            .iter()
            .find(|nn| nn.cell_type as usize >= self.type_names.len())
        {
            bail!("neuron {} has cell type out of range", bad.root_id);
        }
        Ok(())
    }
}

fn read_u8(r: &mut impl Read) -> Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(r: &mut impl Read) -> Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_f32(r: &mut impl Read) -> Result<f32> {
    Ok(f32::from_le_bytes(read_u32(r)?.to_le_bytes()))
}

fn read_u32_vec(r: &mut impl Read, n: usize) -> Result<Vec<u32>> {
    let mut raw = vec![0u8; n * 4];
    r.read_exact(&mut raw)?;
    Ok(raw
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

fn read_f32_vec(r: &mut impl Read, n: usize) -> Result<Vec<f32>> {
    let mut raw = vec![0u8; n * 4];
    r.read_exact(&mut raw)?;
    Ok(raw
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}
