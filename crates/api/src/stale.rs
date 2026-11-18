// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stale operation markers (architecture rule 9).
//!
//! When a setup's toolpath is planned, each of its operations is stamped with
//! a fingerprint of everything the plan was made from: the part's geometry,
//! the stock, the setup's placement and fixture, the operation's own
//! parameters and the tool it cuts with. Later, an operation whose current
//! fingerprint differs from its stamp is **stale**: its toolpath no longer
//! matches the design. Planning it again stamps it afresh.
//!
//! The stamps live in the session, not in the file: they say what was last
//! looked at on screen, and an operation never planned has no stamp and is
//! never marked.

use cndt_geom::TriangleMesh;
use cndt_model::Document;
use cndt_model::cam::{OperationDef, OperationEntry, SetupEntry};
use serde::Serialize;

/// FNV-1a, 64 bit: a fixed, platform-independent hash. A change detector, not a
/// security measure.
pub struct Fingerprint(u64);

impl Fingerprint {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// An empty fingerprint.
    #[must_use]
    pub fn new() -> Self {
        Self(Self::OFFSET)
    }

    /// Mixes in bytes.
    pub fn bytes(&mut self, data: &[u8]) {
        for byte in data {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    /// Mixes in a value through its JSON text, which is stable for the
    /// model's types (maps are ordered, fields are in declaration order).
    pub fn value<T: Serialize>(&mut self, value: &T) {
        match serde_json::to_vec(value) {
            Ok(bytes) => self.bytes(&bytes),
            // Cannot happen for the model's types; a marker byte keeps the
            // result defined and different from "nothing".
            Err(_) => self.bytes(b"\xff"),
        }
    }

    /// The hash so far.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fingerprint {
    fn default() -> Self {
        Self::new()
    }
}

/// The geometry of a mesh, down to the bit.
#[must_use]
pub fn mesh_fingerprint(mesh: &TriangleMesh) -> u64 {
    let mut hash = Fingerprint::new();
    for p in &mesh.positions {
        hash.bytes(&p.to_le_bytes());
    }
    for i in &mesh.indices {
        hash.bytes(&i.to_le_bytes());
    }
    for f in &mesh.triangle_faces {
        hash.bytes(&f.to_le_bytes());
    }
    hash.finish()
}

fn tool_of(operation: &OperationDef) -> Option<u32> {
    match operation {
        OperationDef::Holes => None,
        OperationDef::Contour { tool, .. }
        | OperationDef::Pocket { tool, .. }
        | OperationDef::Chamfer { tool, .. }
        | OperationDef::Face { tool } => *tool,
    }
}

/// What `entry`'s toolpath is made from. `geometry` is
/// [`mesh_fingerprint`] of the part, or `None` when there is no part.
///
/// An operation with a named tool depends on that tool alone; one that picks
/// its tool from the library (and the hole cycles) depends on the whole
/// library, since a new tool can change the choice.
#[must_use]
pub fn operation_fingerprint(
    document: &Document,
    geometry: Option<u64>,
    setup: &SetupEntry,
    entry: &OperationEntry,
) -> u64 {
    let cam = document.cam();
    let mut hash = Fingerprint::new();
    hash.value(&geometry);
    hash.value(&cam.stock);
    let mut placement = setup.setup.clone();
    placement.operations.clear();
    hash.value(&placement);
    hash.value(&entry.operation);
    match tool_of(&entry.operation) {
        Some(number) => hash.value(&cam.tools.get(&number)),
        None => hash.value(&cam.tools),
    }
    hash.finish()
}
