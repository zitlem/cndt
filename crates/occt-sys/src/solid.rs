// SPDX-License-Identifier: AGPL-3.0-or-later
//! Solid modelling: owned shapes, extrusion, booleans, and the face history
//! each operation reports (decision 0012).

use std::ffi::{c_char, c_int};
use std::ptr::NonNull;

use super::{
    ERROR_BUFFER, Mesh, MeshHandle, OcctError, RawMeshView, copy_array, error_text, lock, status,
};

#[repr(C)]
struct ShapeHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct HistoryHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct RawCurve {
    kind: i32,
    p0: [f64; 3],
    p1: [f64; 3],
    p2: [f64; 3],
    radius: f64,
}

#[repr(C)]
#[derive(Default)]
struct RawProperties {
    volume: f64,
    min: [f64; 3],
    max: [f64; 3],
    faces: u32,
    solids: u32,
}

unsafe extern "C" {
    fn cndt_occt_free_shape(shape: *mut ShapeHandle);
    fn cndt_occt_free_history(history: *mut HistoryHandle);
    fn cndt_occt_history_view(
        history: *const HistoryHandle,
        pairs: *mut *const u32,
        len: *mut usize,
    );
    fn cndt_occt_extrude(
        curves: *const RawCurve,
        curve_count: usize,
        loop_sizes: *const u32,
        loop_count: usize,
        normal: *const f64,
        distance: f64,
        out: *mut *mut ShapeHandle,
        history: *mut *mut HistoryHandle,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_boolean(
        a: *const ShapeHandle,
        b: *const ShapeHandle,
        op: i32,
        out: *mut *mut ShapeHandle,
        history: *mut *mut HistoryHandle,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_properties(
        shape: *const ShapeHandle,
        out: *mut RawProperties,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_mesh_shape(
        shape: *const ShapeHandle,
        deflection: f64,
        handle: *mut *mut MeshHandle,
        view: *mut RawMeshView,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_free_mesh(handle: *mut MeshHandle);
    fn cndt_occt_revolve(
        points: *const f64,
        point_count: usize,
        axis_origin: *const f64,
        axis_dir: *const f64,
        out: *mut *mut ShapeHandle,
        history: *mut *mut HistoryHandle,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_face_plane(
        shape: *const ShapeHandle,
        face: u32,
        point: *mut f64,
        normal: *mut f64,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
}

const MODEL_FAILED: i32 = 6;

/// A profile curve in model coordinates (millimetres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// A straight line.
    Line {
        /// Start point.
        start: [f64; 3],
        /// End point.
        end: [f64; 3],
    },
    /// A circular arc, counter-clockwise about the extrusion normal.
    Arc {
        /// Centre.
        center: [f64; 3],
        /// Start point; sets the radius.
        start: [f64; 3],
        /// End point.
        end: [f64; 3],
    },
    /// A full circle in the plane normal to the extrusion.
    Circle {
        /// Centre.
        center: [f64; 3],
        /// Radius.
        radius: f64,
    },
}

impl Curve {
    fn raw(self) -> RawCurve {
        let zero = [0.0; 3];
        match self {
            Self::Line { start, end } => RawCurve {
                kind: 0,
                p0: start,
                p1: end,
                p2: zero,
                radius: 0.0,
            },
            Self::Arc { center, start, end } => RawCurve {
                kind: 1,
                p0: center,
                p1: start,
                p2: end,
                radius: 0.0,
            },
            Self::Circle { center, radius } => RawCurve {
                kind: 2,
                p0: center,
                p1: zero,
                p2: zero,
                radius,
            },
        }
    }
}

/// A shape owned by OpenCascade, freed when dropped.
#[derive(Debug)]
pub struct Shape {
    ptr: NonNull<ShapeHandle>,
}

// SAFETY: a `cndt_shape` is a plain C++ value with no thread affinity, and
// every access to it, including the free in `Drop`, holds the global
// OpenCascade lock.
unsafe impl Send for Shape {}
// SAFETY: as above; shared references only pass the pointer to shim calls
// made under the global lock, which never mutate the shape.
unsafe impl Sync for Shape {}

impl Drop for Shape {
    fn drop(&mut self) {
        let _guard = lock();
        // SAFETY: `ptr` came from a successful shim call that handed over
        // ownership, and `Shape` frees it exactly once.
        unsafe { cndt_occt_free_shape(self.ptr.as_ptr()) };
    }
}

/// How a boolean combines two shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    /// Union.
    Fuse,
    /// The first minus the second.
    Cut,
}

/// What an operation made: the shape, and its history as `(source, face)`
/// pairs. See `cndt_occt_extrude` and `cndt_occt_boolean` for what the
/// sources mean.
#[derive(Debug)]
pub struct Made {
    /// The result.
    pub shape: Shape,
    /// Which result face (0-based) each source became.
    pub history: Vec<(u32, u32)>,
}

/// Volume and size of a shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Properties {
    /// Cubic millimetres.
    pub volume: f64,
    /// The corner with the smallest coordinates.
    pub min: [f64; 3],
    /// The corner with the largest coordinates.
    pub max: [f64; 3],
    /// Number of faces.
    pub faces: u32,
    /// Number of solids.
    pub solids: u32,
}

fn status_error(code: c_int, err: &[u8]) -> OcctError {
    match code {
        MODEL_FAILED => OcctError::ModelFailed(error_text(err)),
        status::MESH_FAILED => OcctError::MeshFailed(error_text(err)),
        _ => OcctError::Kernel(error_text(err)),
    }
}

/// Takes ownership of a shape and history the shim handed out.
///
/// # Safety
///
/// `shape` and `history` must come from one successful shim call, and the
/// caller must hold the OpenCascade lock.
unsafe fn take(shape: *mut ShapeHandle, history: *mut HistoryHandle) -> Result<Made, OcctError> {
    let Some(ptr) = NonNull::new(shape) else {
        return Err(OcctError::Kernel(String::from(
            "the kernel returned no shape",
        )));
    };
    let shape = Shape { ptr };
    let mut pairs: *const u32 = std::ptr::null();
    let mut len = 0_usize;
    if !history.is_null() {
        // SAFETY: `history` is a live history handle (caller's guarantee);
        // the out-pointers are valid locals.
        unsafe { cndt_occt_history_view(history, &mut pairs, &mut len) };
    }
    // SAFETY: the view points into `history`'s vector, of exactly `len`
    // values, and `history` is freed only after this copy.
    let flat = unsafe { copy_array(pairs, len) };
    if !history.is_null() {
        // SAFETY: `history` is owned here and freed exactly once.
        unsafe { cndt_occt_free_history(history) };
    }
    let history = flat
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[source, face]| (source, face))
        .collect();
    Ok(Made { shape, history })
}

/// Sweeps a planar region along `normal` by `distance` millimetres (negative
/// sweeps the other way). The first loop is the outer boundary and the rest
/// are holes; each loop may run either way round.
///
/// History sources: curve `i` (counting through all loops in order) for the
/// side face it swept; `n` for the start cap and `n + 1` for the end cap.
///
/// # Errors
///
/// [`OcctError::ModelFailed`] with a reason when the profile or the sweep is
/// not valid.
pub fn extrude(loops: &[Vec<Curve>], normal: [f64; 3], distance: f64) -> Result<Made, OcctError> {
    let curves: Vec<RawCurve> = loops.iter().flatten().map(|c| c.raw()).collect();
    let sizes: Vec<u32> = loops
        .iter()
        .map(|l| u32::try_from(l.len()).unwrap_or(u32::MAX))
        .collect();
    let mut shape: *mut ShapeHandle = std::ptr::null_mut();
    let mut history: *mut HistoryHandle = std::ptr::null_mut();
    let mut err = [0_u8; ERROR_BUFFER];
    let _guard = lock();
    // SAFETY: `curves`, `sizes` and `normal` are live arrays of the stated
    // lengths whose element layouts match the C declarations. The out-
    // pointers are valid locals. `err` is writable for `err.len()` bytes.
    // The shim catches every C++ exception.
    let code = unsafe {
        cndt_occt_extrude(
            curves.as_ptr(),
            curves.len(),
            sizes.as_ptr(),
            sizes.len(),
            normal.as_ptr(),
            distance,
            &mut shape,
            &mut history,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    if code != status::OK {
        return Err(status_error(code, &err));
    }
    // SAFETY: the call succeeded, so both handles are ours; the lock is held.
    unsafe { take(shape, history) }
}

/// Revolves a closed polygon a full turn about an axis. The polygon lies in a
/// plane through the axis and may touch it along an edge.
///
/// History sources: edge `i`, from point `i` to the next, for the face it
/// swept; edges on the axis sweep none.
///
/// # Errors
///
/// [`OcctError::ModelFailed`] with a reason when the profile is not valid.
pub fn revolve(
    points: &[[f64; 3]],
    axis_origin: [f64; 3],
    axis_dir: [f64; 3],
) -> Result<Made, OcctError> {
    let flat: Vec<f64> = points.iter().flatten().copied().collect();
    let mut shape: *mut ShapeHandle = std::ptr::null_mut();
    let mut history: *mut HistoryHandle = std::ptr::null_mut();
    let mut err = [0_u8; ERROR_BUFFER];
    let _guard = lock();
    // SAFETY: `flat` holds `points.len()` points of three doubles; the axis
    // arrays hold three doubles each; the out-pointers are valid locals;
    // `err` is writable for `err.len()` bytes; the shim catches exceptions.
    let code = unsafe {
        cndt_occt_revolve(
            flat.as_ptr(),
            points.len(),
            axis_origin.as_ptr(),
            axis_dir.as_ptr(),
            &mut shape,
            &mut history,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    if code != status::OK {
        return Err(status_error(code, &err));
    }
    // SAFETY: the call succeeded, so both handles are ours; the lock is held.
    unsafe { take(shape, history) }
}

/// Fuses or cuts two shapes.
///
/// History sources: face `i` of `a` is `i`; face `j` of `b` is
/// `faces(a) + j`.
///
/// # Errors
///
/// [`OcctError::ModelFailed`] with a reason, such as a cut that removes
/// everything.
pub fn boolean(a: &Shape, b: &Shape, op: BooleanOp) -> Result<Made, OcctError> {
    let mut shape: *mut ShapeHandle = std::ptr::null_mut();
    let mut history: *mut HistoryHandle = std::ptr::null_mut();
    let mut err = [0_u8; ERROR_BUFFER];
    let op = match op {
        BooleanOp::Fuse => 0,
        BooleanOp::Cut => 1,
    };
    let _guard = lock();
    // SAFETY: `a` and `b` are live shapes; the out-pointers are valid locals;
    // `err` is writable for `err.len()` bytes; the shim catches exceptions.
    let code = unsafe {
        cndt_occt_boolean(
            a.ptr.as_ptr(),
            b.ptr.as_ptr(),
            op,
            &mut shape,
            &mut history,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    if code != status::OK {
        return Err(status_error(code, &err));
    }
    // SAFETY: the call succeeded, so both handles are ours; the lock is held.
    unsafe { take(shape, history) }
}

/// Volume, bounds and counts of a shape.
///
/// # Errors
///
/// [`OcctError::Kernel`] if OpenCascade fails.
pub fn properties(shape: &Shape) -> Result<Properties, OcctError> {
    let mut raw = RawProperties::default();
    let mut err = [0_u8; ERROR_BUFFER];
    let _guard = lock();
    // SAFETY: `shape` is live; `raw` matches the C layout; `err` is writable
    // for `err.len()` bytes; the shim catches exceptions.
    let code = unsafe {
        cndt_occt_properties(
            shape.ptr.as_ptr(),
            &mut raw,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    if code != status::OK {
        return Err(status_error(code, &err));
    }
    Ok(Properties {
        volume: raw.volume,
        min: raw.min,
        max: raw.max,
        faces: raw.faces,
        solids: raw.solids,
    })
}

/// Triangulates a shape for display. Faces are numbered as in the history.
///
/// # Errors
///
/// [`OcctError::MeshFailed`] if triangulation fails.
pub fn mesh(shape: &Shape, deflection: f64) -> Result<Mesh, OcctError> {
    let mut handle: *mut MeshHandle = std::ptr::null_mut();
    let mut view = RawMeshView::default();
    let mut err = [0_u8; ERROR_BUFFER];
    let _guard = lock();
    // SAFETY: `shape` is live; `handle` and `view` are valid out-pointers
    // matching the C layouts; `err` is writable for `err.len()` bytes; the
    // shim catches exceptions.
    let code = unsafe {
        cndt_occt_mesh_shape(
            shape.ptr.as_ptr(),
            deflection,
            &mut handle,
            &mut view,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    if code != status::OK || handle.is_null() {
        return Err(status_error(code, &err));
    }
    // SAFETY: on success `view` points into arrays owned by `handle`, with
    // their exact lengths; `handle` is freed only after the copies.
    let mesh = unsafe {
        Mesh {
            positions: copy_array(view.positions, view.positions_len),
            indices: copy_array(view.indices, view.indices_len),
            triangle_faces: copy_array(view.triangle_faces, view.triangle_faces_len),
            face_count: view.face_count,
        }
    };
    // SAFETY: `handle` is owned here and freed exactly once.
    unsafe { cndt_occt_free_mesh(handle) };
    Ok(mesh)
}

/// A point on a flat face, and its outward normal.
///
/// # Errors
///
/// [`OcctError::ModelFailed`] if the face is not flat; [`OcctError::Kernel`]
/// if there is no such face.
pub fn face_plane(shape: &Shape, face: u32) -> Result<([f64; 3], [f64; 3]), OcctError> {
    let mut point = [0.0; 3];
    let mut normal = [0.0; 3];
    let mut err = [0_u8; ERROR_BUFFER];
    let _guard = lock();
    // SAFETY: `shape` is live; `point` and `normal` are writable arrays of
    // three doubles; `err` is writable for `err.len()` bytes; the shim
    // catches every C++ exception.
    let code = unsafe {
        cndt_occt_face_plane(
            shape.ptr.as_ptr(),
            face,
            point.as_mut_ptr(),
            normal.as_mut_ptr(),
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    if code != status::OK {
        return Err(status_error(code, &err));
    }
    Ok((point, normal))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    fn rectangle(w: f64, h: f64) -> Vec<Curve> {
        let p = [[0.0, 0.0, 0.0], [w, 0.0, 0.0], [w, h, 0.0], [0.0, h, 0.0]];
        (0..4)
            .map(|i| Curve::Line {
                start: p[i],
                end: p[(i + 1) % 4],
            })
            .collect()
    }

    fn circle(x: f64, y: f64, r: f64) -> Vec<Curve> {
        vec![Curve::Circle {
            center: [x, y, 0.0],
            radius: r,
        }]
    }

    const Z: [f64; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn extrude_a_block_and_name_every_face() {
        let made = extrude(&[rectangle(96.0, 56.0)], Z, 20.0).unwrap();
        let props = properties(&made.shape).unwrap();
        assert!(
            (props.volume - 96.0 * 56.0 * 20.0).abs() < 1e-6,
            "{props:?}"
        );
        assert_eq!((props.faces, props.solids), (6, 1));
        // Four sides from four lines, plus two caps: every face once.
        let mut faces: Vec<u32> = made.history.iter().map(|p| p.1).collect();
        faces.sort_unstable();
        assert_eq!(faces, [0, 1, 2, 3, 4, 5]);
        let sources: Vec<u32> = made.history.iter().map(|p| p.0).collect();
        assert_eq!(sources, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_hole_and_an_arc_profile() {
        // A plate with a through hole: rectangle, then a clockwise hole.
        let made = extrude(&[rectangle(40.0, 30.0), circle(20.0, 15.0, 5.0)], Z, 10.0).unwrap();
        let props = properties(&made.shape).unwrap();
        let expected = 40.0 * 30.0 * 10.0 - std::f64::consts::PI * 25.0 * 10.0;
        assert!(
            (props.volume - expected).abs() < 1e-3,
            "{} vs {expected}",
            props.volume
        );
        // A half-disc: line and arc.
        let half = vec![
            Curve::Line {
                start: [-5.0, 0.0, 0.0],
                end: [5.0, 0.0, 0.0],
            },
            Curve::Arc {
                center: [0.0, 0.0, 0.0],
                start: [5.0, 0.0, 0.0],
                end: [-5.0, 0.0, 0.0],
            },
        ];
        let made = extrude(&[half], Z, 2.0).unwrap();
        let v = properties(&made.shape).unwrap().volume;
        assert!((v - std::f64::consts::PI * 25.0).abs() < 1e-3, "{v}");
    }

    #[test]
    fn cut_a_pocket_and_follow_the_faces() {
        let block = extrude(&[rectangle(96.0, 56.0)], Z, 20.0).unwrap();
        let pocket = vec![
            Curve::Line {
                start: [28.0, 18.0, 20.0],
                end: [68.0, 18.0, 20.0],
            },
            Curve::Line {
                start: [68.0, 18.0, 20.0],
                end: [68.0, 38.0, 20.0],
            },
            Curve::Line {
                start: [68.0, 38.0, 20.0],
                end: [28.0, 38.0, 20.0],
            },
            Curve::Line {
                start: [28.0, 38.0, 20.0],
                end: [28.0, 18.0, 20.0],
            },
        ];
        let tool = extrude(&[pocket], Z, -8.0).unwrap();
        let cut = boolean(&block.shape, &tool.shape, BooleanOp::Cut).unwrap();
        let props = properties(&cut.shape).unwrap();
        assert!((props.volume - (96.0 * 56.0 * 20.0 - 40.0 * 20.0 * 8.0)).abs() < 1e-6);
        // 6 block faces + 4 pocket walls + pocket floor.
        assert_eq!(props.faces, 11);
        // The block's four sides and bottom carry over; its top becomes the
        // top with a hole; the tool's top cap is gone.
        let from_block = cut.history.iter().filter(|p| p.0 < 6).count();
        assert_eq!(from_block, 6, "{:?}", cut.history);
        let mesh = mesh(&cut.shape, 0.1).unwrap();
        assert_eq!(mesh.face_count, 11);
        assert!(!mesh.indices.is_empty());
    }

    #[test]
    fn revolve_a_drilled_hole_shape() {
        // Half-section of a Ø10 × 20 hole with a 118° point, axis along -Z.
        let r = 5.0;
        let tip = r / (59f64.to_radians()).tan();
        let profile = [
            [0.0, 0.0, 0.0],
            [r, 0.0, 0.0],
            [r, 0.0, -20.0],
            [0.0, 0.0, -20.0 - tip],
        ];
        let made = revolve(&profile, [0.0; 3], [0.0, 0.0, -1.0]).unwrap();
        let props = properties(&made.shape).unwrap();
        let expected = std::f64::consts::PI * r * r * (20.0 + tip / 3.0);
        assert!(
            (props.volume - expected).abs() < 1e-6,
            "{} vs {expected}",
            props.volume
        );
        // Top, wall and tip faces from edges 0, 1, 2; the axis edge sweeps none.
        let mut sources: Vec<u32> = made.history.iter().map(|p| p.0).collect();
        sources.sort_unstable();
        assert_eq!(sources, [0, 1, 2], "{:?}", made.history);
        assert_eq!(props.faces, 3);
    }

    #[test]
    fn face_planes_point_outwards() {
        let made = extrude(&[rectangle(10.0, 10.0)], Z, 5.0).unwrap();
        let (_, end) = *made.history.iter().find(|p| p.0 == 5).unwrap();
        let (point, normal) = face_plane(&made.shape, end).unwrap();
        assert!(
            (point[2] - 5.0).abs() < 1e-9 && (normal[2] - 1.0).abs() < 1e-12,
            "{point:?} {normal:?}"
        );
        let (_, start) = *made.history.iter().find(|p| p.0 == 4).unwrap();
        let (_, normal) = face_plane(&made.shape, start).unwrap();
        assert!(
            (normal[2] + 1.0).abs() < 1e-12,
            "the bottom faces down: {normal:?}"
        );
        let disc = extrude(&[circle(0.0, 0.0, 3.0)], Z, 2.0).unwrap();
        let (_, side) = *disc.history.iter().find(|p| p.0 == 0).unwrap();
        assert!(matches!(
            face_plane(&disc.shape, side),
            Err(OcctError::ModelFailed(_))
        ));
        assert!(face_plane(&disc.shape, 99).is_err());
    }

    #[test]
    fn failures_explain_themselves() {
        let open = vec![Curve::Line {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
        }];
        let error = extrude(&[open], Z, 1.0).unwrap_err();
        assert!(
            matches!(error, OcctError::ModelFailed(_) | OcctError::Kernel(_)),
            "{error:?}"
        );
        let a = extrude(&[rectangle(10.0, 10.0)], Z, 5.0).unwrap();
        let b = extrude(&[rectangle(10.0, 10.0)], Z, 5.0).unwrap();
        let error = boolean(&a.shape, &b.shape, BooleanOp::Cut).unwrap_err();
        assert!(
            error.to_string().contains("whole body") || error.to_string().contains("no solid"),
            "{error}"
        );
    }

    #[test]
    fn stacked_extrusions_merge_into_one_clean_solid() {
        let low = extrude(&[rectangle(20.0, 20.0)], Z, 5.0).unwrap();
        let high: Vec<Curve> = rectangle(20.0, 20.0)
            .into_iter()
            .map(|c| match c {
                Curve::Line { start, end } => Curve::Line {
                    start: [start[0], start[1], 5.0],
                    end: [end[0], end[1], 5.0],
                },
                other => other,
            })
            .collect();
        let high = extrude(&[high], Z, 5.0).unwrap();
        let fused = boolean(&low.shape, &high.shape, BooleanOp::Fuse).unwrap();
        let props = properties(&fused.shape).unwrap();
        assert!((props.volume - 4000.0).abs() < 1e-6);
        assert_eq!(props.faces, 6, "coplanar sides merge");
    }
}
