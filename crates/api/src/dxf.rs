// SPDX-License-Identifier: AGPL-3.0-or-later
//! DXF export (R12 text, millimetres): a sketch exactly, or the top or
//! bottom face of the part with its holes, for cutting on 2D machines.

use std::fmt::Write as _;
use std::path::Path;

use cndt_geom::Point3;
use cndt_kernel::OutlineSegment;
use cndt_model::sketch::Geometry;

use crate::{Api, ApiError};

/// A 2D curve for the file.
enum Curve {
    Line([f64; 2], [f64; 2]),
    /// Centre, radius, start and end angle in degrees, counter-clockwise.
    Arc([f64; 2], f64, f64, f64),
    Circle([f64; 2], f64),
}

/// How a number is written: six decimals, deterministic.
fn n(value: f64) -> String {
    let text = format!("{value:.6}");
    if text == "-0.000000" {
        String::from("0.000000")
    } else {
        text
    }
}

fn arc(center: [f64; 2], start: [f64; 2], end: [f64; 2], ccw: bool) -> Curve {
    let radius = (start[0] - center[0]).hypot(start[1] - center[1]);
    if (start[0] - end[0]).hypot(start[1] - end[1]) < 1e-9 {
        return Curve::Circle(center, radius);
    }
    let angle = |p: [f64; 2]| {
        (p[1] - center[1])
            .atan2(p[0] - center[0])
            .to_degrees()
            .rem_euclid(360.0)
    };
    // DXF arcs always run counter-clockwise.
    let (from, to) = if ccw { (start, end) } else { (end, start) };
    Curve::Arc(center, radius, angle(from), angle(to))
}

fn write(curves: &[(Curve, &str)]) -> String {
    let mut out = String::new();
    let mut pair = |code: u32, value: &str| {
        let _ = writeln!(out, "{code:>3}\n{value}");
    };
    pair(0, "SECTION");
    pair(2, "HEADER");
    pair(9, "$ACADVER");
    pair(1, "AC1009");
    // Millimetres.
    pair(9, "$INSUNITS");
    pair(70, "4");
    pair(9, "$MEASUREMENT");
    pair(70, "1");
    pair(0, "ENDSEC");
    pair(0, "SECTION");
    pair(2, "ENTITIES");
    for (curve, layer) in curves {
        match curve {
            Curve::Line(a, b) => {
                pair(0, "LINE");
                pair(8, layer);
                for (code, value) in [
                    (10, a[0]),
                    (20, a[1]),
                    (30, 0.0),
                    (11, b[0]),
                    (21, b[1]),
                    (31, 0.0),
                ] {
                    pair(code, &n(value));
                }
            }
            Curve::Arc(c, r, from, to) => {
                pair(0, "ARC");
                pair(8, layer);
                for (code, value) in [
                    (10, c[0]),
                    (20, c[1]),
                    (30, 0.0),
                    (40, *r),
                    (50, *from),
                    (51, *to),
                ] {
                    pair(code, &n(value));
                }
            }
            Curve::Circle(c, r) => {
                pair(0, "CIRCLE");
                pair(8, layer);
                for (code, value) in [(10, c[0]), (20, c[1]), (30, 0.0), (40, *r)] {
                    pair(code, &n(value));
                }
            }
        }
    }
    pair(0, "ENDSEC");
    pair(0, "EOF");
    out
}

impl Api {
    /// Writes a DXF of a sketch (in its own coordinates; construction
    /// geometry on layer CONSTRUCTION) or of the part cut through at model
    /// height `section` (its true walls there, outside and holes, on layer
    /// PROFILE, seen from above). Replies `{entities}`.
    pub(crate) fn export_dxf(
        &self,
        path: &Path,
        sketch: Option<u64>,
        section: Option<f64>,
    ) -> Result<serde_json::Value, ApiError> {
        let curves = match (sketch, section) {
            (Some(id), None) => self.sketch_curves(id)?,
            (None, Some(z)) => self.section_curves(z)?,
            _ => {
                return Err(ApiError::new(
                    "bad_request",
                    "give either sketch or section (a height)",
                ));
            }
        };
        if curves.is_empty() {
            return Err(ApiError::new("bad_request", "there is nothing to export"));
        }
        std::fs::write(path, write(&curves))
            .map_err(|e| ApiError::new("io", format!("cannot write {}: {e}", path.display())))?;
        Ok(serde_json::json!({ "entities": curves.len() }))
    }

    fn sketch_curves(&self, id: u64) -> Result<Vec<(Curve, &'static str)>, ApiError> {
        let sketch_id = u32::try_from(id)
            .map(cndt_model::SketchId)
            .map_err(|_| ApiError::new("bad_request", format!("there is no sketch {id}")))?;
        let record = self
            .session
            .document()
            .sketch(sketch_id)
            .ok_or_else(|| ApiError::new("bad_request", format!("there is no sketch {id}")))?;
        let sketch = &record.sketch;
        let at = |e| sketch.point(e).map(|p| [p.x, p.y]);
        let mut curves = Vec::new();
        for (_, entity) in sketch.entities() {
            let layer = if entity.construction {
                "CONSTRUCTION"
            } else {
                "SKETCH"
            };
            let curve = match entity.geometry {
                Geometry::Point { .. } => None,
                Geometry::Line { start, end } => {
                    at(start).zip(at(end)).map(|(a, b)| Curve::Line(a, b))
                }
                Geometry::Circle { center, radius } => at(center).map(|c| Curve::Circle(c, radius)),
                Geometry::Arc { center, start, end } => match (at(center), at(start), at(end)) {
                    (Some(c), Some(s), Some(e)) => Some(arc(c, s, e, true)),
                    _ => None,
                },
            };
            if let Some(curve) = curve {
                curves.push((curve, layer));
            }
        }
        Ok(curves)
    }

    fn section_curves(&self, z: f64) -> Result<Vec<(Curve, &'static str)>, ApiError> {
        let body = self
            .rebuilt
            .body
            .as_ref()
            .ok_or_else(|| ApiError::new("no_body", "there is no solid to export yet"))?;
        let xy = |p: Point3| [p.x, p.y];
        let mut curves = Vec::new();
        for outline in self.kernel.section_loops(body, z)? {
            for segment in outline {
                curves.push((
                    match segment {
                        OutlineSegment::Line { start, end } => Curve::Line(xy(start), xy(end)),
                        OutlineSegment::Arc {
                            start,
                            end,
                            center,
                            ccw,
                        } => arc(xy(center), xy(start), xy(end), ccw),
                    },
                    "PROFILE",
                ));
            }
        }
        Ok(curves)
    }
}
