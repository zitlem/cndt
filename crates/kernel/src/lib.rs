// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 1 boundary to the B-rep geometry kernel.
//!
//! This is the only crate allowed to use OpenCascade. Every other crate uses
//! the [`Kernel`] trait and the types defined here, so the kernel can be
//! replaced later (decision 0007, architecture rule 3). No OpenCascade type
//! appears in this crate's public API.

use std::fmt;
use std::path::{Path, PathBuf};

use cndt_geom::{BoundingBox, Point3};

/// Topology counts and size of an imported shape.
#[derive(Debug, Clone, PartialEq)]
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
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, reason } => {
                write!(f, "cannot read {}: {reason}", path.display())
            }
            Self::Empty { path } => write!(f, "{} contains no geometry", path.display()),
            Self::Internal(reason) => write!(f, "geometry kernel error: {reason}"),
        }
    }
}

impl std::error::Error for KernelError {}

/// The operations CNDT needs from a geometry kernel.
pub trait Kernel {
    /// The kernel's name and version, such as `OpenCascade 7.9.3`.
    fn description(&self) -> String;

    /// Imports a STEP file and summarises its topology and size.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError`] if the file cannot be read, holds no geometry,
    /// or the kernel fails while processing it.
    fn import_step(&self, path: &Path) -> Result<ImportSummary, KernelError>;
}

/// The OpenCascade implementation of [`Kernel`].
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCascade;

impl Kernel for OpenCascade {
    fn description(&self) -> String {
        format!("OpenCascade {}", cndt_occt_sys::version())
    }

    fn import_step(&self, path: &Path) -> Result<ImportSummary, KernelError> {
        use cndt_occt_sys::OcctError;

        let summary = cndt_occt_sys::read_step(path).map_err(|error| match error {
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
        })?;

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
    fn describes_itself() {
        assert!(OpenCascade.description().starts_with("OpenCascade 7"));
    }
}
