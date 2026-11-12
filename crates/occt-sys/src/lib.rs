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

unsafe extern "C" {
    fn cndt_occt_version() -> *const c_char;
    fn cndt_occt_read_step(
        path: *const c_char,
        out: *mut RawSummary,
        err: *mut c_char,
        err_len: usize,
    ) -> c_int;
}

/// Status codes returned by the shim. Keep in step with `cpp/occt_shim.cpp`.
mod status {
    pub const OK: i32 = 0;
    pub const READ_FAILED: i32 = 1;
    pub const NO_SHAPES: i32 = 2;
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
}

impl fmt::Display for OcctError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid path: {path}"),
            Self::ReadFailed(reason) => write!(f, "read failed: {reason}"),
            Self::NoShapes => write!(f, "the file contains no shapes"),
            Self::Kernel(reason) => write!(f, "geometry kernel error: {reason}"),
        }
    }
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
    fn a_non_step_file_is_an_error_not_a_crash() {
        let result = read_step(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
        assert!(result.is_err(), "got {result:?}");
    }
}
