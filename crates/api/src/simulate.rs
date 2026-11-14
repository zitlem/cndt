// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stock simulation of one setup (decision 0016): the setup's program swept
//! through its raw stock, checked against the exact part.

use std::collections::BTreeMap;

use cndt_geom::{Point3, TriangleMesh};
use cndt_model::cam::{Side, ToolKind};
use cndt_sim::ToolShape;

use crate::{Api, ApiError};

/// The default cell size, in millimetres.
pub const DEFAULT_CELL: f64 = 0.25;

/// A simulation kept for drawing, with what it was made from.
#[derive(Debug)]
pub(crate) struct Simulated {
    revision: u64,
    setup: Option<u32>,
    report: cndt_sim::Report,
    part: Vec<Option<f64>>,
    placed: cndt_process_mill::Setup,
}

/// The colour classes of the simulated stock mesh, as its "faces".
pub const STOCK_CLASSES: u32 = 4;

impl Api {
    /// Simulates one setup and reports collisions, gouges, material left
    /// and cutting time.
    pub(crate) fn simulate_setup(
        &self,
        setup: Option<u32>,
        cell: Option<f64>,
    ) -> Result<Simulated, ApiError> {
        let cell = cell.unwrap_or(DEFAULT_CELL);
        if !(cell.is_finite() && (0.05..=5.0).contains(&cell)) {
            return Err(ApiError::new(
                "bad_request",
                "the cell size must be 0.05 to 5 mm",
            ));
        }
        let document = self.session.document();
        let cam_error = |e: cndt_process_mill::CamError| ApiError::new("cam", e.to_string());
        let planned =
            cndt_process_mill::plan_setup(document, &self.rebuilt, self.kernel.as_ref(), setup)
                .map_err(cam_error)?;
        let Some(placed) = planned.setup else {
            return Err(ApiError::new("cam", "the setup has no placement"));
        };
        let stock =
            cndt_process_mill::stock_box(document, &self.rebuilt, &placed).map_err(cam_error)?;
        let body = self
            .rebuilt
            .body
            .as_ref()
            .ok_or_else(|| ApiError::new("no_body", "there is no solid yet"))?;
        let grid = cndt_sim::Grid::over(stock.min.x, stock.min.y, stock.max.x, stock.max.y, cell);
        // The part's top in each cell, seen along the spindle: lines
        // through the cell centres in model coordinates, back to program Z.
        let from_above = placed.side == Side::Top;
        let model_points: Vec<[f64; 2]> = grid
            .centres()
            .iter()
            .map(|[x, y]| {
                let m = placed.model_point(Point3::new(*x, *y, 0.0));
                [m.x, m.y]
            })
            .collect();
        let hits = self.kernel.heights(body, &model_points, from_above)?;
        let part = hits
            .iter()
            .zip(&model_points)
            .map(|(hit, [x, y])| hit.map(|z| placed.program_point(Point3::new(*x, *y, z)).z))
            .collect();
        let tools = document
            .cam()
            .tools
            .iter()
            .map(|(number, tool)| (*number, shape(&tool.kind, tool.diameter)))
            .collect::<BTreeMap<_, _>>();
        let input = cndt_sim::Setup {
            grid,
            top: stock.max.z,
            bottom: stock.min.z,
            part,
        };
        let report = cndt_sim::simulate(&planned.program, &tools, &input)
            .map_err(|e| ApiError::new("simulation", e.to_string()))?;
        Ok(Simulated {
            revision: self.revision,
            setup,
            report,
            part: input.part,
            placed,
        })
    }

    /// The simulated stock of a setup for the 3D view, in model
    /// coordinates, in the layout of [`crate::MESH_MAGIC`]. Each triangle's
    /// "face" is its class: 0 machined to the part, 1 material left on the
    /// part, 2 cut below the part (a gouge), 3 stock off the part. Uses the
    /// last `simulate` of the same setup if nothing changed since.
    ///
    /// # Errors
    ///
    /// As the `simulate` request.
    pub fn simulation_mesh(&self, setup: Option<u32>) -> Result<Vec<u8>, ApiError> {
        let fresh;
        let sim = match &self.simulated {
            Some(kept) if kept.revision == self.revision && kept.setup == setup => kept,
            _ => {
                fresh = self.simulate_setup(setup, Some(0.5))?;
                &fresh
            }
        };
        crate::encode_mesh(&stock_mesh(sim))
    }

    /// Keeps a simulation for [`Api::simulation_mesh`].
    pub(crate) fn keep_simulation(&mut self, sim: Simulated) -> &cndt_sim::Report {
        &self.simulated.insert(sim).report
    }
}

/// Triangles between the cell centres of the simulated stock. Cells cut
/// through to the stock's bottom off the part are left out.
fn stock_mesh(sim: &Simulated) -> TriangleMesh {
    let grid = sim.report.grid;
    let heights = &sim.report.heights;
    let bottom = heights.iter().copied().fold(f64::MAX, f64::min);
    let class = |c: usize| -> u32 {
        match sim.part[c] {
            Some(top) if heights[c] < top - cndt_sim::TOLERANCE => 2,
            Some(top) if heights[c] > top + 0.05 => 1,
            Some(_) => 0,
            None => 3,
        }
    };
    let empty = |c: usize| sim.part[c].is_none() && heights[c] <= bottom + 1e-9;
    let mut mesh = TriangleMesh {
        positions: Vec::with_capacity(heights.len() * 3),
        indices: Vec::new(),
        triangle_faces: Vec::new(),
        face_count: STOCK_CLASSES,
    };
    for ([x, y], h) in grid.centres().iter().zip(heights) {
        let m = sim.placed.model_point(Point3::new(*x, *y, *h));
        #[allow(clippy::cast_possible_truncation)]
        mesh.positions.extend([m.x as f32, m.y as f32, m.z as f32]);
    }
    let flipped = sim.placed.side == Side::Bottom;
    for j in 0..grid.ny.saturating_sub(1) {
        for i in 0..grid.nx.saturating_sub(1) {
            let a = j * grid.nx + i;
            let (b, c, d) = (a + 1, a + grid.nx + 1, a + grid.nx);
            for tri in [[a, b, c], [a, c, d]] {
                if tri.iter().all(|&v| empty(v)) {
                    continue;
                }
                // Facing the spindle: turned over, the winding flips.
                let tri = if flipped {
                    [tri[0], tri[2], tri[1]]
                } else {
                    tri
                };
                let worst = tri.iter().map(|&v| class(v)).fold(0, |w, k| {
                    // A gouge shows over material left, which shows over clean.
                    let rank = |k: u32| [0, 1, 3, 2][k as usize];
                    if rank(k) > rank(w) { k } else { w }
                });
                #[allow(clippy::cast_possible_truncation)]
                mesh.indices.extend(tri.map(|v| v as u32));
                mesh.triangle_faces.push(worst);
            }
        }
    }
    mesh
}

/// The cutting shape of a library tool. A tap is simulated at its tap
/// drill's diameter: the model has no thread form to compare with.
fn shape(kind: &ToolKind, diameter: f64) -> ToolShape {
    match kind {
        ToolKind::SpotDrill | ToolKind::ChamferMill => ToolShape::Cone {
            diameter,
            angle: 90.0,
        },
        ToolKind::Drill => ToolShape::Cone {
            diameter,
            angle: 118.0,
        },
        ToolKind::Tap { thread } => ToolShape::Flat {
            diameter: cndt_model::threads::thread(thread)
                .map_or(diameter, cndt_model::threads::ThreadSize::tap_drill),
        },
        ToolKind::Reamer | ToolKind::EndMill | ToolKind::FaceMill => ToolShape::Flat { diameter },
    }
}
