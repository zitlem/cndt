// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 0 geometry primitives: points, vectors, units and tolerances.
//!
//! All lengths are millimetres stored as `f64`. Inch exists only as a display
//! and post-output setting. See `docs/architecture.md`, rules 10 and 11.

use serde::{Deserialize, Serialize};

/// A point in model space. Coordinates are millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point3 {
    /// X coordinate in millimetres.
    pub x: f64,
    /// Y coordinate in millimetres.
    pub y: f64,
    /// Z coordinate in millimetres.
    pub z: f64,
}

impl Point3 {
    /// Creates a point from coordinates in millimetres.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

/// An axis-aligned bounding box in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    /// The corner with the smallest coordinates.
    pub min: Point3,
    /// The corner with the largest coordinates.
    pub max: Point3,
}

impl BoundingBox {
    /// Extent along X, Y and Z in millimetres.
    #[must_use]
    pub fn size(&self) -> [f64; 3] {
        [
            self.max.x - self.min.x,
            self.max.y - self.min.y,
            self.max.z - self.min.z,
        ]
    }
}

/// A triangle mesh for display. Lengths are millimetres.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TriangleMesh {
    /// Vertex positions, three values (x, y, z) per vertex.
    pub positions: Vec<f32>,
    /// Vertex indices, three per triangle, counter-clockwise seen from outside.
    pub indices: Vec<u32>,
    /// For each triangle, the index of the model face it belongs to.
    pub triangle_faces: Vec<u32>,
    /// How many model faces there are.
    pub face_count: u32,
}

impl TriangleMesh {
    /// Number of vertices.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_is_max_minus_min() {
        let bounds = BoundingBox {
            min: Point3::new(-1.0, 2.0, 0.5),
            max: Point3::new(9.0, 22.0, 30.5),
        };
        let size = bounds.size();
        let expected = [10.0, 20.0, 30.0];
        for axis in 0..3 {
            assert!((size[axis] - expected[axis]).abs() < 1e-12, "{size:?}");
        }
    }
}
