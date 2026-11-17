// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exporting the part: STEP for other CAD and CAM, STL for meshes and
//! printing. The part as built at the history marker is what is written.

use std::path::Path;

use crate::{Api, ApiError};

/// The tessellation tolerance for STL, in millimetres: fine enough that the
/// facets do not show on a machined part's curves.
const STL_DEFLECTION: f64 = 0.01;

impl Api {
    /// Writes the part to `path`, as STEP (`.step`, `.stp`) or binary STL
    /// (`.stl`) by the extension. Replies with the format and, for STL, the
    /// triangle count.
    pub(crate) fn export(&self, path: &Path) -> Result<serde_json::Value, ApiError> {
        let body = self
            .rebuilt
            .body
            .as_ref()
            .ok_or_else(|| ApiError::new("no_body", "there is no solid to export yet"))?;
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("step" | "stp") => {
                self.kernel.write_step(body, path)?;
                Ok(serde_json::json!({ "format": "step" }))
            }
            Some("stl") => {
                let mesh = self.kernel.mesh(body, STL_DEFLECTION)?;
                let bytes = stl(&mesh, self.session.document().name())?;
                std::fs::write(path, bytes).map_err(|e| {
                    ApiError::new("io", format!("cannot write {}: {e}", path.display()))
                })?;
                Ok(serde_json::json!({ "format": "stl", "triangles": mesh.triangle_count() }))
            }
            _ => Err(ApiError::new(
                "bad_request",
                "export to a .step, .stp or .stl file",
            )),
        }
    }
}

/// A binary STL file: an 80-byte header, the triangle count, then each
/// triangle's normal and corners as 32-bit floats and two spare bytes.
fn stl(mesh: &cndt_geom::TriangleMesh, name: &str) -> Result<Vec<u8>, ApiError> {
    let count = u32::try_from(mesh.triangle_count())
        .map_err(|_| ApiError::new("kernel", "the mesh is too large for STL"))?;
    let mut out = Vec::with_capacity(84 + 50 * mesh.triangle_count());
    let mut header = format!("CNDT {name}").into_bytes();
    // "solid" at the start would make readers take the file for text STL.
    header.retain(u8::is_ascii);
    header.resize(80, b' ');
    out.extend_from_slice(&header);
    out.extend_from_slice(&count.to_le_bytes());
    let corner = |index: u32| -> [f32; 3] {
        let i = index as usize * 3;
        [
            mesh.positions.get(i).copied().unwrap_or_default(),
            mesh.positions.get(i + 1).copied().unwrap_or_default(),
            mesh.positions.get(i + 2).copied().unwrap_or_default(),
        ]
    };
    let (triangles, _) = mesh.indices.as_chunks::<3>();
    for triangle in triangles {
        let [a, b, c] = [
            corner(triangle[0]),
            corner(triangle[1]),
            corner(triangle[2]),
        ];
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let normal = if length > 0.0 {
            [n[0] / length, n[1] / length, n[2] / length]
        } else {
            [0.0; 3]
        };
        for value in normal.iter().chain(&a).chain(&b).chain(&c) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    Ok(out)
}
