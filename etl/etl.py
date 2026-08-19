#!/usr/bin/env python3
"""Build a .gnat connectome from FlyWire CSV exports.

STATUS: placeholder. The original DesktopFly ships its own etl.py and its own
neuron subset; this reproduces the *shape* of that step so the Rust side has a
real file to load, but the subset selection and the weight scaling still have
to be reconciled against the original. See README, "porting status".

Expected inputs (FlyWire Codex exports, --data-dir):

  connections.csv     pre_root_id, post_root_id, syn_count, nt_type
  classification.csv  root_id, super_class, cell_type
  coordinates.csv     root_id, position          (optional; used for the brain view)

Output is the little-endian .gnat format read by crates/gnat-sim/src/connectome.rs.
"""

from __future__ import annotations

import argparse
import csv
import struct
import sys
from collections import defaultdict
from pathlib import Path

MAGIC = b"GNAT"
VERSION = 1

ROLE_INTERNEURON, ROLE_SENSORY, ROLE_MOTOR = 0, 1, 2

# FlyWire super_class values mapped onto the three roles the sim cares about.
SUPER_CLASS_ROLE = {
    "sensory": ROLE_SENSORY,
    "visual_projection": ROLE_SENSORY,
    "optic": ROLE_SENSORY,
    "ascending": ROLE_SENSORY,
    "motor": ROLE_MOTOR,
    "descending": ROLE_MOTOR,
    "endocrine": ROLE_MOTOR,
}

# Sign applied to a synapse's weight, by neurotransmitter. Anything unlisted is
# treated as excitatory, which is the FlyWire convention for unknowns.
NT_SIGN = {
    "gaba": -1.0,
    "glut": -1.0,
    "ach": 1.0,
    "ser": 1.0,
    "oct": 1.0,
    "da": 1.0,
    "dopamine": 1.0,
    "unk": 1.0,
}


def read_csv(path: Path) -> list[dict[str, str]]:
    if not path.exists():
        return []
    with path.open(newline="") as f:
        return list(csv.DictReader(f))


def load_classification(data_dir: Path) -> tuple[dict[int, int], dict[int, str]]:
    """root_id -> role, and root_id -> cell type name."""
    roles: dict[int, int] = {}
    types: dict[int, str] = {}
    for row in read_csv(data_dir / "classification.csv"):
        try:
            rid = int(row["root_id"])
        except (KeyError, ValueError):
            continue
        sc = (row.get("super_class") or "").strip().lower()
        roles[rid] = SUPER_CLASS_ROLE.get(sc, ROLE_INTERNEURON)
        types[rid] = (row.get("cell_type") or sc or "unknown").strip() or "unknown"
    return roles, types


def load_positions(data_dir: Path) -> dict[int, tuple[float, float, float]]:
    """root_id -> a single representative point, for the brain view only."""
    out: dict[int, tuple[float, float, float]] = {}
    for row in read_csv(data_dir / "coordinates.csv"):
        try:
            rid = int(row["root_id"])
        except (KeyError, ValueError):
            continue
        raw = (row.get("position") or "").strip().strip("[]()")
        parts = [p for p in raw.replace(",", " ").split() if p]
        if len(parts) < 3:
            continue
        try:
            out.setdefault(rid, (float(parts[0]), float(parts[1]), float(parts[2])))
        except ValueError:
            continue
    return out


def load_edges(data_dir: Path, min_syn: int) -> list[tuple[int, int, float]]:
    """(pre, post, signed weight), dropping synapses below the noise floor."""
    edges = []
    path = data_dir / "connections.csv"
    if not path.exists():
        sys.exit(f"missing {path}")
    for row in read_csv(path):
        try:
            pre = int(row["pre_root_id"])
            post = int(row["post_root_id"])
            count = int(row["syn_count"])
        except (KeyError, ValueError):
            continue
        if count < min_syn:
            continue
        nt = (row.get("nt_type") or "").strip().lower()
        edges.append((pre, post, count * NT_SIGN.get(nt, 1.0)))
    return edges


def write_gnat(
    out: Path,
    order: list[int],
    roles: dict[int, int],
    types: dict[int, str],
    positions: dict[int, tuple[float, float, float]],
    edges: list[tuple[int, int, float]],
) -> None:
    index = {rid: i for i, rid in enumerate(order)}

    type_names: list[str] = []
    type_index: dict[str, int] = {}
    for rid in order:
        name = types.get(rid, "unknown")
        if name not in type_index:
            type_index[name] = len(type_names)
            type_names.append(name)

    # Group edges by presynaptic neuron, in neuron-index order, for CSR.
    by_pre: dict[int, list[tuple[int, float]]] = defaultdict(list)
    for pre, post, w in edges:
        by_pre[index[pre]].append((index[post], w))

    offsets, targets, weights = [0], [], []
    for i in range(len(order)):
        for post, w in sorted(by_pre.get(i, ())):
            targets.append(post)
            weights.append(w)
        offsets.append(len(targets))

    with out.open("wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<III", VERSION, len(order), len(targets)))
        f.write(struct.pack("<I", len(type_names)))
        for name in type_names:
            raw = name.encode()
            f.write(struct.pack("<I", len(raw)))
            f.write(raw)
        for rid in order:
            x, y, z = positions.get(rid, (0.0, 0.0, 0.0))
            f.write(struct.pack("<Q", rid))
            f.write(struct.pack("<B", roles.get(rid, ROLE_INTERNEURON)))
            f.write(struct.pack("<I", type_index[types.get(rid, "unknown")]))
            f.write(struct.pack("<fff", x, y, z))
        f.write(struct.pack(f"<{len(offsets)}I", *offsets))
        if targets:
            f.write(struct.pack(f"<{len(targets)}I", *targets))
            f.write(struct.pack(f"<{len(weights)}f", *weights))

    print(
        f"wrote {out}: {len(order)} neurons, {len(targets)} synapses, "
        f"{len(type_names)} cell types"
    )


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--data-dir", type=Path, default=Path("data/raw"))
    ap.add_argument("--out", type=Path, default=Path("data/brain.gnat"))
    ap.add_argument(
        "--min-syn",
        type=int,
        default=5,
        help="drop connections with fewer synapses than this (FlyWire's own noise floor)",
    )
    args = ap.parse_args()

    roles, types = load_classification(args.data_dir)
    positions = load_positions(args.data_dir)
    edges = load_edges(args.data_dir, args.min_syn)

    # Only keep neurons that actually take part in a surviving connection;
    # an isolated cell is 30 bytes of file and zero behaviour.
    connected = {pre for pre, _, _ in edges} | {post for _, post, _ in edges}
    order = sorted(connected)
    edges = [e for e in edges if e[0] in connected and e[1] in connected]

    args.out.parent.mkdir(parents=True, exist_ok=True)
    write_gnat(args.out, order, roles, types, positions, edges)


if __name__ == "__main__":
    main()
