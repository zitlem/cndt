// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 0 geometry primitives: points, vectors, units and tolerances.
//!
//! All lengths are millimetres stored as `f64`. Inch exists only as a display
//! and post-output setting. See `docs/architecture.md`, rules 10 and 11.

pub mod stl;

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

/// A point in a 2D plane, such as a sketch plane. Coordinates are millimetres.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point2 {
    /// X coordinate in millimetres.
    pub x: f64,
    /// Y coordinate in millimetres.
    pub y: f64,
}

impl Point2 {
    /// Creates a point from coordinates in millimetres.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Straight-line distance to another point, in millimetres.
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        (other.x - self.x).hypot(other.y - self.y)
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

    #[test]
    fn distance_is_symmetric_and_zero_to_itself() {
        let a = Point2::new(1.0, 2.0);
        let b = Point2::new(4.0, 6.0);
        assert!((a.distance(b) - 5.0).abs() < 1e-12);
        assert!((b.distance(a) - 5.0).abs() < 1e-12);
        assert!(a.distance(a).abs() < 1e-12);
    }

    #[test]
    fn mesh_counts_follow_the_buffers() {
        let empty = TriangleMesh::default();
        assert_eq!((empty.vertex_count(), empty.triangle_count()), (0, 0));
        let one = TriangleMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            triangle_faces: vec![0],
            face_count: 1,
        };
        assert_eq!((one.vertex_count(), one.triangle_count()), (3, 1));
    }

    #[test]
    fn a_box_with_negative_extent_reports_it() {
        // size() does not clamp: a swapped box shows up as negative, not zero.
        let swapped = BoundingBox {
            min: Point3::new(5.0, 0.0, 0.0),
            max: Point3::new(1.0, 0.0, 0.0),
        };
        assert!((swapped.size()[0] + 4.0).abs() < 1e-12);
    }
}
