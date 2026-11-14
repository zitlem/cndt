// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 0 bridge to OpenCascade.
//!
//! This crate holds the project's only `unsafe` code. It exposes a small,
//! safe Rust API over a C++ shim (`cpp/occt_shim.cpp`) that catches every
//! C++ exception, so nothing unwinds across the language boundary.
//!
//! Only `cndt-kernel` may depend on this crate; the layer check enforces it.
//! Everything else goes through the kernel's trait (decision 0007).
//!
//! # Threads
//!
//! OpenCascade's data exchange code (STEP, IGES) keeps global state and is
//! not safe to call from several threads at once: concurrent reads crashed
//! the process in testing. Every call into the shim therefore holds
//! [`OCCT_LOCK`]. This serialises kernel work; parallelism, where it is
//! wanted later, has to come from inside OpenCascade or from separate
//! processes.

mod solid;

pub use solid::{
    Blend, BooleanOp, Curve, Made, Properties, Segment, Shape, blend, boolean, edge_polylines,
    edges, extrude, face_loops, face_outline, face_plane, heights, mesh, properties, revolve,
    section_outline, translate,
};

use std::ffi::{CStr, CString, c_char, c_int};
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Held for the whole of every call into OpenCascade. See the crate docs.
static OCCT_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    // A poisoned lock only means another thread panicked while holding it.
    // The shim keeps no Rust state, so carrying on is safe.
    OCCT_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Layout must match `cndt_shape_summary` in `cpp/occt_shim.cpp`.
#[repr(C)]
#[derive(Default)]
struct RawSummary {
    solids: u32,
    shells: u32,
    faces: u32,
    edges: u32,
    vertices: u32,
    min: [f64; 3],
    max: [f64; 3],
    has_bounds: u8,
}

/// The shim's mesh object. Only ever handled through a pointer.
#[repr(C)]
struct MeshHandle {
    _private: [u8; 0],
}

/// Layout must match `cndt_mesh_view` in `cpp/occt_shim.cpp`.
#[repr(C)]
struct RawMeshView {
    positions: *const f32,
    positions_len: usize,
    indices: *const u32,
    indices_len: usize,
    triangle_faces: *const u32,
    triangle_faces_len: usize,
    face_count: u32,
}

impl Default for RawMeshView {
    fn default() -> Self {
        Self {
            positions: std::ptr::null(),
            positions_len: 0,
            indices: std::ptr::null(),
            indices_len: 0,
            triangle_faces: std::ptr::null(),
            triangle_faces_len: 0,
            face_count: 0,
        }
    }
}

unsafe extern "C" {
    fn cndt_occt_version() -> *const c_char;
    fn cndt_occt_read_step(
        path: *const c_char,
        out: *mut RawSummary,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_mesh_step(
        path: *const c_char,
        deflection: f64,
        handle: *mut *mut MeshHandle,
        view: *mut RawMeshView,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
    fn cndt_occt_free_mesh(handle: *mut MeshHandle);
}

/// Status codes returned by the shim. Keep in step with `cpp/occt_shim.cpp`.
mod status {
    pub const OK: i32 = 0;
    pub const READ_FAILED: i32 = 1;
    pub const NO_SHAPES: i32 = 2;
    pub const MESH_FAILED: i32 = 5;
}

/// A triangle mesh for display. Lengths are millimetres.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    /// Vertex positions, three values (x, y, z) per vertex.
    pub positions: Vec<f32>,
    /// Vertex indices, three per triangle, wound counter-clockwise seen from
    /// outside the solid.
    pub indices: Vec<u32>,
    /// For each triangle, the index of the face it belongs to.
    pub triangle_faces: Vec<u32>,
    /// How many faces the shape has.
    pub face_count: u32,
}

/// Copies `len` items starting at `ptr` into a vector.
///
/// # Safety
///
/// `ptr` must point to `len` initialised items that stay valid for the call,
/// or `len` must be zero.
unsafe fn copy_array<T: Copy>(ptr: *const T, len: usize) -> Vec<T> {
    if len == 0 || ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: guaranteed by the caller.
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

/// Size of the buffer the shim writes error messages into.
const ERROR_BUFFER: usize = 512;

/// Topology counts and bounds of a shape. Lengths are millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSummary {
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
    /// Axis-aligned bounds as `(min, max)`, or `None` for an empty shape.
    pub bounds: Option<([f64; 3], [f64; 3])>,
}

/// Why a call into OpenCascade failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcctError {
    /// The path cannot be passed to OpenCascade (not UTF-8, or contains NUL).
    InvalidPath(String),
    /// The file could not be read as the requested format.
    ReadFailed(String),
    /// The file was read but held no shapes.
    NoShapes,
    /// OpenCascade raised an error while working on the shape.
    Kernel(String),
    /// The shape could not be triangulated.
    MeshFailed(String),
    /// A modelling operation could not produce a valid solid; says why.
    ModelFailed(String),
}

impl fmt::Display for OcctError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::ReadFailed(reason) => write!(f, "read failed: {reason}"),
            Self::NoShapes => write!(f, "the file contains no shapes"),
            Self::Kernel(reason) => write!(f, "geometry kernel error: {reason}"),
            Self::MeshFailed(reason) => write!(f, "triangulation failed: {reason}"),
            Self::ModelFailed(reason) => f.write_str(reason),
        }
    }
}

fn c_path(path: &Path) -> Result<CString, OcctError> {
    let text = path
        .to_str()
        .ok_or_else(|| OcctError::InvalidPath(path.display().to_string()))?;
    CString::new(text).map_err(|_| OcctError::InvalidPath(text.to_owned()))
}

fn error_text(err: &[u8]) -> String {
    CStr::from_bytes_until_nul(err)
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl std::error::Error for OcctError {}

/// The OpenCascade version this build is linked against, such as `7.9.3`.
#[must_use]
pub fn version() -> String {
    let _guard = lock();
    // SAFETY: the shim returns a pointer to a static NUL-terminated string
    // literal, or null. It takes no arguments and cannot fail.
    let ptr = unsafe { cndt_occt_version() };
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: `ptr` is non-null and points to a static NUL-terminated string
    // that lives for the whole program.
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Reads a STEP file and summarises its topology and bounds.
///
/// # Errors
///
/// Returns an error if the path is not valid UTF-8, the file is not readable
/// STEP, it holds no shapes, or OpenCascade raises an error.
pub fn read_step(path: &Path) -> Result<ShapeSummary, OcctError> {
    let text = path
        .to_str()
        .ok_or_else(|| OcctError::InvalidPath(path.display().to_string()))?;
    let c_path = CString::new(text).map_err(|_| OcctError::InvalidPath(text.to_owned()))?;
    let mut summary = RawSummary::default();
    let mut err = [0_u8; ERROR_BUFFER];

    let guard = lock();
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the
    // call. `summary` is a valid, exclusively borrowed `RawSummary` whose
    // layout matches the C struct. `err` is a writable buffer of exactly
    // `err.len()` bytes, and the shim never writes past that length. The shim
    // catches all C++ exceptions, so the call cannot unwind.
    let code = unsafe {
        cndt_occt_read_step(
            c_path.as_ptr(),
            &mut summary,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    drop(guard);

    let message = || {
        CStr::from_bytes_until_nul(&err)
            .map(|text| text.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match code {
        status::OK => Ok(ShapeSummary {
            solids: summary.solids,
            shells: summary.shells,
            faces: summary.faces,
            edges: summary.edges,
            vertices: summary.vertices,
            bounds: (summary.has_bounds != 0).then_some((summary.min, summary.max)),
        }),
        status::READ_FAILED => Err(OcctError::ReadFailed(message())),
        status::NO_SHAPES => Err(OcctError::NoShapes),
        _ => Err(OcctError::Kernel(message())),
    }
}

/// Reads a STEP file and triangulates it for display.
///
/// `deflection` is the largest allowed distance, in millimetres, between the
/// triangles and the true surface. Every face has its own vertices, so
/// normals stay sharp across face edges. The result is the same on every run.
///
/// # Errors
///
/// Returns an error if the path is invalid, the file is not readable STEP,
/// it holds no shapes, triangulation fails, or `deflection` is not positive.
pub fn mesh_step(path: &Path, deflection: f64) -> Result<Mesh, OcctError> {
    let c_path = c_path(path)?;
    let mut handle: *mut MeshHandle = std::ptr::null_mut();
    let mut view = RawMeshView::default();
    let mut err = [0_u8; ERROR_BUFFER];

    let guard = lock();
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the
    // call. `handle` and `view` are valid, exclusively borrowed out-pointers
    // whose layouts match the C declarations. `err` is a writable buffer of
    // exactly `err.len()` bytes. The shim catches all C++ exceptions.
    let code = unsafe {
        cndt_occt_mesh_step(
            c_path.as_ptr(),
            deflection,
            &mut handle,
            &mut view,
            err.as_mut_ptr().cast::<c_char>(),
            err.len(),
        )
    };
    let result = if code == status::OK && !handle.is_null() {
        // SAFETY: on success the shim filled `view` with pointers into the
        // arrays owned by `handle`, with their exact lengths. `handle` is not
        // freed until after these copies.
        let mesh = unsafe {
            Mesh {
                positions: copy_array(view.positions, view.positions_len),
                indices: copy_array(view.indices, view.indices_len),
                triangle_faces: copy_array(view.triangle_faces, view.triangle_faces_len),
                face_count: view.face_count,
            }
        };
        // SAFETY: `handle` came from a successful `cndt_occt_mesh_step` and is
        // freed exactly once; nothing points into it after this.
        unsafe { cndt_occt_free_mesh(handle) };
        Ok(mesh)
    } else {
        Err(match code {
            status::READ_FAILED => OcctError::ReadFailed(error_text(&err)),
            status::NO_SHAPES => OcctError::NoShapes,
            status::MESH_FAILED => OcctError::MeshFailed(error_text(&err)),
            _ => OcctError::Kernel(error_text(&err)),
        })
    };
    drop(guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn reports_a_version() {
        assert!(version().starts_with('7'), "got {:?}", version());
    }

    #[test]
    fn reads_the_box_fixture() {
        let summary = read_step(&fixture("box-10x20x30.step")).unwrap();
        assert_eq!(
            (
                summary.solids,
                summary.shells,
                summary.faces,
                summary.edges,
                summary.vertices
            ),
            (1, 1, 6, 12, 8)
        );
        let (min, max) = summary.bounds.unwrap();
        let expected_max = [10.0, 20.0, 30.0];
        for axis in 0..3 {
            assert!(close(min[axis], 0.0), "min {min:?}");
            assert!(close(max[axis], expected_max[axis]), "max {max:?}");
        }
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_crash() {
        let result = read_step(&fixture("does-not-exist.step"));
        assert!(
            matches!(result, Err(OcctError::ReadFailed(_))),
            "got {result:?}"
        );
    }

    /// Regression test: unserialised concurrent reads crashed the process
    /// about once in a few hundred test runs.
    #[test]
    fn concurrent_reads_are_safe() {
        let path = fixture("box-10x20x30.step");
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        assert_eq!(read_step(&path).unwrap().faces, 6);
                        assert!(read_step(Path::new("missing.step")).is_err());
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn meshes_the_box_with_outward_triangles() {
        let mesh = mesh_step(&fixture("box-10x20x30.step"), 0.05).unwrap();
        assert_eq!(mesh.face_count, 6);
        assert_eq!(mesh.indices.len(), 12 * 3, "a box is 12 triangles");
        assert_eq!(mesh.triangle_faces.len(), 12);
        assert_eq!(mesh.positions.len() % 3, 0);

        let point = |index: u32| {
            let i = index as usize * 3;
            [
                f64::from(mesh.positions[i]),
                f64::from(mesh.positions[i + 1]),
                f64::from(mesh.positions[i + 2]),
            ]
        };
        let centre = [5.0, 10.0, 15.0];
        for triangle in mesh.indices.chunks(3) {
            let [a, b, c] = [point(triangle[0]), point(triangle[1]), point(triangle[2])];
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let outward = [a[0] - centre[0], a[1] - centre[1], a[2] - centre[2]];
            let dot = normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
            assert!(dot > 0.0, "triangle {triangle:?} faces inwards");
        }
        for value in &mesh.positions {
            assert!((-1e-4..=30.0001).contains(value), "{value} outside the box");
        }
    }

    #[test]
    fn meshing_is_deterministic_and_rejects_bad_input() {
        let path = fixture("box-10x20x30.step");
        assert_eq!(
            mesh_step(&path, 0.05).unwrap(),
            mesh_step(&path, 0.05).unwrap()
        );
        assert!(mesh_step(&path, 0.0).is_err());
        assert!(matches!(
            mesh_step(&fixture("missing.step"), 0.05),
            Err(OcctError::ReadFailed(_))
        ));
    }

    #[test]
    fn a_non_step_file_is_an_error_not_a_crash() {
        let result = read_step(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
        assert!(result.is_err(), "got {result:?}");
    }
}
