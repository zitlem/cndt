// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 1 boundary to the B-rep geometry kernel.
//!
//! This is the only crate allowed to use OpenCascade. Every other crate uses
//! the [`Kernel`] trait and the types defined here, so the kernel can be
//! replaced later (decision 0007, architecture rule 3). No OpenCascade type
//! appears in this crate's public API.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cndt_geom::{BoundingBox, Point3, TriangleMesh};
use serde::Serialize;

/// Topology counts and size of an imported shape.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportSummary {
    /// Number of distinct solids.
    pub solids: u32,
    /// Number of distinct shells.
    pub shells: u32,
    /// Number of distinct faces.
    pub faces: u32,
    /// Number of distinct edges.
    pub edges: u32,
    /// Number of distinct vertices.
    pub vertices: u32,
    /// Axis-aligned bounds, or `None` for an empty shape.
    pub bounds: Option<BoundingBox>,
}

/// Why a kernel operation failed. Every variant carries a readable cause
/// (architecture rule 14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// The file could not be read.
    Unreadable {
        /// The file that was requested.
        path: PathBuf,
        /// What went wrong, in words a user can act on.
        reason: String,
    },
    /// The file was read but contained no geometry.
    Empty {
        /// The file that was requested.
        path: PathBuf,
    },
    /// The kernel failed while working on valid input.
    Internal(String),
    /// A modelling operation could not make a valid solid. Says why, in
    /// words a user can act on.
    Modelling(String),
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, reason } => {
                write!(f, "cannot read {}: {reason}", path.display())
            }
            Self::Empty { path } => write!(f, "{} contains no geometry", path.display()),
            Self::Internal(reason) => write!(f, "geometry kernel error: {reason}"),
            Self::Modelling(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for KernelError {}

/// The operations CNDT needs from a geometry kernel.
pub trait Kernel: fmt::Debug + Send + Sync {
    /// The kernel's name and version, such as `OpenCascade 7.9.3`.
    fn description(&self) -> String;

    /// Imports a STEP file and summarises its topology and size.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError`] if the file cannot be read, holds no geometry,
    /// or the kernel fails while processing it.
    fn import_step(&self, path: &Path) -> Result<ImportSummary, KernelError>;

    /// Triangulates a STEP file for display. `deflection` is the largest
    /// allowed distance, in millimetres, between triangles and the surface.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError`] if the file cannot be read, holds no geometry,
    /// or cannot be triangulated.
    fn mesh_step(&self, path: &Path, deflection: f64) -> Result<TriangleMesh, KernelError>;

    /// Sweeps a planar region along `normal` by `distance` millimetres
    /// (negative sweeps backwards). The first loop is the outer boundary, the
    /// rest are holes; loops may run either way round, but each must be in
    /// order, every curve starting where the last ended.
    ///
    /// History sources: curve `i`, counting through all loops, for the side
    /// face it swept; `n` for the start cap and `n + 1` for the end cap.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] when the profile or sweep is not valid.
    fn extrude(
        &self,
        loops: &[Vec<Curve>],
        normal: [f64; 3],
        distance: f64,
    ) -> Result<Made, KernelError>;

    /// Revolves a closed polygon a full turn about an axis. The polygon lies
    /// in a plane through the axis and may touch it along an edge.
    ///
    /// History sources: edge `i`, from point `i` to the next, for the face it
    /// swept; edges on the axis sweep none.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] when the profile is not valid.
    fn revolve(
        &self,
        points: &[Point3],
        axis_origin: Point3,
        axis_dir: [f64; 3],
    ) -> Result<Made, KernelError>;

    /// For every edge of a body: the two faces it joins (face numbers as in
    /// the history; a seam lists its one face twice). Edge numbers are the
    /// positions in this list.
    ///
    /// # Errors
    ///
    /// [`KernelError::Internal`] if the kernel fails.
    fn edges(&self, body: &Body) -> Result<Vec<(u32, u32)>, KernelError>;

    /// Every edge of a body as a polyline (x, y, z per point) within
    /// `deflection` of the curve, in the order [`Kernel::edges`] uses.
    ///
    /// # Errors
    ///
    /// [`KernelError::Internal`] if the kernel fails.
    fn edge_polylines(&self, body: &Body, deflection: f64) -> Result<Vec<Vec<f32>>, KernelError>;

    /// The outer boundary of flat face `face` (parallel to XY), offset by
    /// `distance`: outwards when positive, inwards when negative, the
    /// boundary itself at 0. Lines and arcs in running order.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] if the offset is not one closed loop
    /// (inwards: the outline has vanished or split).
    fn face_outline(
        &self,
        body: &Body,
        face: u32,
        distance: f64,
    ) -> Result<Vec<OutlineSegment>, KernelError>;

    /// The outline of the body cut through at height `z` by a plane parallel
    /// to XY: the section's outer loop, offset outwards by `distance`. Lines
    /// and arcs in running order. Unlike a face's outline, this is the true
    /// wall whatever chamfers or fillets do at the faces above and below.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] if there is no closed outline at `z`, or it
    /// cannot be offset into one loop.
    fn section_outline(
        &self,
        body: &Body,
        z: f64,
        distance: f64,
    ) -> Result<Vec<OutlineSegment>, KernelError>;

    /// Every boundary loop of flat face `face` (parallel to XY), offset with
    /// the face by `distance`: positive grows the face, so the outer loop
    /// moves outwards and holes shrink.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] if the offset fails.
    fn face_loops(
        &self,
        body: &Body,
        face: u32,
        distance: f64,
    ) -> Result<Vec<Vec<OutlineSegment>>, KernelError>;

    /// Chamfers or fillets edges of a body.
    ///
    /// History sources: input face `i` is `i`; the face made from the `k`-th
    /// listed edge is `faces + k`.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] with a reason, such as a blend too large
    /// for the faces around it.
    fn blend(&self, body: &Body, edges: &[u32], blend: Blend) -> Result<Made, KernelError>;

    /// A copy of a body moved by `offset` millimetres. Its faces are
    /// numbered as the original's.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] if the copy cannot be made.
    fn translate(&self, body: &Body, offset: [f64; 3]) -> Result<Body, KernelError>;

    /// Fuses or cuts two bodies.
    ///
    /// History sources: face `i` of `a` is `i`; face `j` of `b` is
    /// `a.faces + j`.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`], for example when a cut removes everything.
    fn boolean(&self, a: &Body, b: &Body, op: BooleanOp) -> Result<Made, KernelError>;

    /// Volume, bounds and face count of a body.
    ///
    /// # Errors
    ///
    /// [`KernelError::Internal`] if the kernel fails.
    fn properties(&self, body: &Body) -> Result<BodyProperties, KernelError>;

    /// Triangulates a body for display. Face numbers match the history.
    ///
    /// # Errors
    ///
    /// [`KernelError::Internal`] if triangulation fails.
    fn mesh(&self, body: &Body, deflection: f64) -> Result<TriangleMesh, KernelError>;

    /// A point on flat face `face` of a body (numbered as in the history)
    /// and its outward unit normal.
    ///
    /// # Errors
    ///
    /// [`KernelError::Modelling`] if the face is not flat.
    fn face_plane(&self, body: &Body, face: u32) -> Result<(Point3, [f64; 3]), KernelError>;
}

/// A profile curve in model coordinates, millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// A straight line.
    Line {
        /// Start.
        start: Point3,
        /// End.
        end: Point3,
    },
    /// An arc, counter-clockwise about the extrusion normal.
    Arc {
        /// Centre.
        center: Point3,
        /// Start; sets the radius.
        start: Point3,
        /// End.
        end: Point3,
    },
    /// A full circle in the plane normal to the extrusion.
    Circle {
        /// Centre.
        center: Point3,
        /// Radius.
        radius: f64,
    },
}

/// What a blend does to an edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blend {
    /// Bevels it, the same distance back on both faces (a 45° chamfer).
    Chamfer(f64),
    /// Rounds it to a radius.
    Fillet(f64),
}

/// One piece of an outline, in model coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutlineSegment {
    /// A straight line.
    Line {
        /// Start.
        start: Point3,
        /// End.
        end: Point3,
    },
    /// A circular arc.
    Arc {
        /// Start.
        start: Point3,
        /// End.
        end: Point3,
        /// Centre.
        center: Point3,
        /// Counter-clockwise about +Z.
        ccw: bool,
    },
}

/// How a boolean combines two bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    /// Union.
    Fuse,
    /// The first minus the second.
    Cut,
}

/// A solid body held by the kernel. Cheap to clone; the geometry is shared.
#[derive(Debug, Clone)]
pub struct Body(Arc<cndt_occt_sys::Shape>);

/// The result of a modelling operation.
#[derive(Debug, Clone)]
pub struct Made {
    /// The new body.
    pub body: Body,
    /// `(source, face)` pairs: which result face each source became. What a
    /// source is depends on the operation.
    pub history: Vec<(u32, u32)>,
}

/// Size and topology of a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyProperties {
    /// Cubic millimetres.
    pub volume: f64,
    /// Bounding box.
    pub bounds: BoundingBox,
    /// Number of faces.
    pub faces: u32,
    /// Number of separate solids.
    pub solids: u32,
}

fn xyz(p: Point3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn modelling(error: cndt_occt_sys::OcctError) -> KernelError {
    match error {
        cndt_occt_sys::OcctError::ModelFailed(reason) => KernelError::Modelling(reason),
        other => KernelError::Internal(other.to_string()),
    }
}

fn made(made: cndt_occt_sys::Made) -> Made {
    Made {
        body: Body(Arc::new(made.shape)),
        history: made.history,
    }
}

/// The OpenCascade implementation of [`Kernel`].
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCascade;

impl Kernel for OpenCascade {
    fn description(&self) -> String {
        format!("OpenCascade {}", cndt_occt_sys::version())
    }

    fn import_step(&self, path: &Path) -> Result<ImportSummary, KernelError> {
        let summary = cndt_occt_sys::read_step(path).map_err(|error| kernel_error(path, error))?;

        Ok(ImportSummary {
            solids: summary.solids,
            shells: summary.shells,
            faces: summary.faces,
            edges: summary.edges,
            vertices: summary.vertices,
            bounds: summary.bounds.map(|(min, max)| BoundingBox {
                min: Point3::new(min[0], min[1], min[2]),
                max: Point3::new(max[0], max[1], max[2]),
            }),
        })
    }

    fn mesh_step(&self, path: &Path, deflection: f64) -> Result<TriangleMesh, KernelError> {
        let mesh = cndt_occt_sys::mesh_step(path, deflection)
            .map_err(|error| kernel_error(path, error))?;
        Ok(TriangleMesh {
            positions: mesh.positions,
            indices: mesh.indices,
            triangle_faces: mesh.triangle_faces,
            face_count: mesh.face_count,
        })
    }

    fn extrude(
        &self,
        loops: &[Vec<Curve>],
        normal: [f64; 3],
        distance: f64,
    ) -> Result<Made, KernelError> {
        use cndt_occt_sys::Curve as C;
        let loops: Vec<Vec<C>> = loops
            .iter()
            .map(|curves| {
                curves
                    .iter()
                    .map(|c| match *c {
                        Curve::Line { start, end } => C::Line {
                            start: xyz(start),
                            end: xyz(end),
                        },
                        Curve::Arc { center, start, end } => C::Arc {
                            center: xyz(center),
                            start: xyz(start),
                            end: xyz(end),
                        },
                        Curve::Circle { center, radius } => C::Circle {
                            center: xyz(center),
                            radius,
                        },
                    })
                    .collect()
            })
            .collect();
        cndt_occt_sys::extrude(&loops, normal, distance)
            .map(made)
            .map_err(modelling)
    }

    fn revolve(
        &self,
        points: &[Point3],
        axis_origin: Point3,
        axis_dir: [f64; 3],
    ) -> Result<Made, KernelError> {
        let points: Vec<[f64; 3]> = points.iter().copied().map(xyz).collect();
        cndt_occt_sys::revolve(&points, xyz(axis_origin), axis_dir)
            .map(made)
            .map_err(modelling)
    }

    fn translate(&self, body: &Body, offset: [f64; 3]) -> Result<Body, KernelError> {
        cndt_occt_sys::translate(&body.0, offset)
            .map(|s| Body(Arc::new(s)))
            .map_err(modelling)
    }

    fn edge_polylines(&self, body: &Body, deflection: f64) -> Result<Vec<Vec<f32>>, KernelError> {
        cndt_occt_sys::edge_polylines(&body.0, deflection).map_err(modelling)
    }

    fn face_outline(
        &self,
        body: &Body,
        face: u32,
        distance: f64,
    ) -> Result<Vec<OutlineSegment>, KernelError> {
        Ok(outline_segments(
            cndt_occt_sys::face_outline(&body.0, face, distance).map_err(modelling)?,
        ))
    }

    fn section_outline(
        &self,
        body: &Body,
        z: f64,
        distance: f64,
    ) -> Result<Vec<OutlineSegment>, KernelError> {
        Ok(outline_segments(
            cndt_occt_sys::section_outline(&body.0, z, distance).map_err(modelling)?,
        ))
    }
    fn face_loops(
        &self,
        body: &Body,
        face: u32,
        distance: f64,
    ) -> Result<Vec<Vec<OutlineSegment>>, KernelError> {
        let p = |a: [f64; 3]| Point3::new(a[0], a[1], a[2]);
        Ok(cndt_occt_sys::face_loops(&body.0, face, distance)
            .map_err(modelling)?
            .into_iter()
            .map(|l| {
                l.into_iter()
                    .map(|s| match s {
                        cndt_occt_sys::Segment::Line { start, end } => OutlineSegment::Line {
                            start: p(start),
                            end: p(end),
                        },
                        cndt_occt_sys::Segment::Arc {
                            start,
                            end,
                            center,
                            ccw,
                        } => OutlineSegment::Arc {
                            start: p(start),
                            end: p(end),
                            center: p(center),
                            ccw,
                        },
                    })
                    .collect()
            })
            .collect())
    }

    fn edges(&self, body: &Body) -> Result<Vec<(u32, u32)>, KernelError> {
        cndt_occt_sys::edges(&body.0).map_err(modelling)
    }

    fn blend(&self, body: &Body, edges: &[u32], blend: Blend) -> Result<Made, KernelError> {
        let blend = match blend {
            Blend::Chamfer(d) => cndt_occt_sys::Blend::Chamfer(d),
            Blend::Fillet(r) => cndt_occt_sys::Blend::Fillet(r),
        };
        cndt_occt_sys::blend(&body.0, edges, blend)
            .map(made)
            .map_err(modelling)
    }

    fn boolean(&self, a: &Body, b: &Body, op: BooleanOp) -> Result<Made, KernelError> {
        let op = match op {
            BooleanOp::Fuse => cndt_occt_sys::BooleanOp::Fuse,
            BooleanOp::Cut => cndt_occt_sys::BooleanOp::Cut,
        };
        cndt_occt_sys::boolean(&a.0, &b.0, op)
            .map(made)
            .map_err(modelling)
    }

    fn properties(&self, body: &Body) -> Result<BodyProperties, KernelError> {
        let p = cndt_occt_sys::properties(&body.0).map_err(modelling)?;
        Ok(BodyProperties {
            volume: p.volume,
            bounds: BoundingBox {
                min: Point3::new(p.min[0], p.min[1], p.min[2]),
                max: Point3::new(p.max[0], p.max[1], p.max[2]),
            },
            faces: p.faces,
            solids: p.solids,
        })
    }

    fn face_plane(&self, body: &Body, face: u32) -> Result<(Point3, [f64; 3]), KernelError> {
        let (p, n) = cndt_occt_sys::face_plane(&body.0, face).map_err(modelling)?;
        Ok((Point3::new(p[0], p[1], p[2]), n))
    }

    fn mesh(&self, body: &Body, deflection: f64) -> Result<TriangleMesh, KernelError> {
        let mesh = cndt_occt_sys::mesh(&body.0, deflection).map_err(modelling)?;
        Ok(TriangleMesh {
            positions: mesh.positions,
            indices: mesh.indices,
            triangle_faces: mesh.triangle_faces,
            face_count: mesh.face_count,
        })
    }
}

fn kernel_error(path: &Path, error: cndt_occt_sys::OcctError) -> KernelError {
    use cndt_occt_sys::OcctError;
    match error {
        OcctError::InvalidPath(_) => KernelError::Unreadable {
            path: path.to_path_buf(),
            reason: String::from("the path is not valid UTF-8"),
        },
        OcctError::ReadFailed(reason) => KernelError::Unreadable {
            path: path.to_path_buf(),
            reason,
        },
        OcctError::NoShapes => KernelError::Empty {
            path: path.to_path_buf(),
        },
        OcctError::Kernel(reason) => KernelError::Internal(reason),
        OcctError::MeshFailed(reason) => {
            KernelError::Internal(format!("triangulation failed: {reason}"))
        }
        OcctError::ModelFailed(reason) => KernelError::Modelling(reason),
    }
}

/// Kernel outline segments from the shim's.
fn outline_segments(segments: Vec<cndt_occt_sys::Segment>) -> Vec<OutlineSegment> {
    let p = |a: [f64; 3]| Point3::new(a[0], a[1], a[2]);
    segments
        .into_iter()
        .map(|s| match s {
            cndt_occt_sys::Segment::Line { start, end } => OutlineSegment::Line {
                start: p(start),
                end: p(end),
            },
            cndt_occt_sys::Segment::Arc {
                start,
                end,
                center,
                ccw,
            } => OutlineSegment::Arc {
                start: p(start),
                end: p(end),
                center: p(center),
                ccw,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn imports_the_box_through_the_trait() {
        let kernel: &dyn Kernel = &OpenCascade;
        let summary = kernel.import_step(&fixture("box-10x20x30.step")).unwrap();
        assert_eq!(summary.solids, 1);
        assert_eq!(summary.faces, 6);
        let size = summary.bounds.unwrap().size();
        let expected = [10.0, 20.0, 30.0];
        for axis in 0..3 {
            assert!((size[axis] - expected[axis]).abs() < 1e-6, "{size:?}");
        }
    }

    #[test]
    fn a_missing_file_names_the_file() {
        let path = fixture("missing.step");
        let error = OpenCascade.import_step(&path).unwrap_err();
        assert!(matches!(error, KernelError::Unreadable { .. }), "{error:?}");
        assert!(error.to_string().contains("missing.step"), "{error}");
    }

    #[test]
    fn meshes_the_box_through_the_trait() {
        let kernel: &dyn Kernel = &OpenCascade;
        let mesh = kernel
            .mesh_step(&fixture("box-10x20x30.step"), 0.05)
            .unwrap();
        assert_eq!(mesh.face_count, 6);
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn describes_itself() {
        assert!(OpenCascade.description().starts_with("OpenCascade 7"));
    }
}
