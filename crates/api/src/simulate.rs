// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stock simulation of one setup (decision 0016): the setup's program swept
//! through its raw stock, checked against the exact part.

use std::collections::BTreeMap;

use cndt_geom::{Point3, TriangleMesh};
use cndt_model::cam::{FlipAxis, Side, ToolKind};
use cndt_sim::ToolShape;

use crate::{Api, ApiError};

/// The default cell size, in millimetres.
pub const DEFAULT_CELL: f64 = 0.25;

/// A simulation kept for drawing, with what it was made from.
#[derive(Debug)]
pub(crate) struct Simulated {
    revision: u64,
    setup: Option<u32>,
    pub(crate) report: cndt_sim::Report,
    part: Vec<Option<f64>>,
    floors: Vec<f64>,
    placed: cndt_process_mill::Setup,
}

/// The colour classes of the simulated stock mesh, as its "faces".
pub const STOCK_CLASSES: u32 = 4;

impl Api {
    /// Simulates one setup and reports collisions, gouges, material left
    /// and cutting time. The setups in `after` (by default, those before it
    /// in the document) are simulated first, so it starts from the stock
    /// they left; an empty `after` starts from a fresh block.
    pub(crate) fn simulate_setup(
        &self,
        setup: Option<u32>,
        cell: Option<f64>,
        after: Option<&[u32]>,
    ) -> Result<Simulated, ApiError> {
        let setups = &self.session.document().cam().setups;
        let target = match setup {
            Some(id) => id,
            None => setups
                .first()
                .map(|s| s.id)
                .ok_or_else(|| ApiError::new("cam", "the document has no setups yet"))?,
        };
        let Some(position) = setups.iter().position(|s| s.id == target) else {
            return Err(ApiError::new("cam", format!("there is no setup {target}")));
        };
        let mut ids: Vec<u32> = match after {
            Some(before) => before.to_vec(),
            None => setups[..position].iter().map(|s| s.id).collect(),
        };
        if ids.contains(&target) {
            return Err(ApiError::new(
                "bad_request",
                format!("setup {target} cannot come after itself"),
            ));
        }
        ids.push(target);
        let mut chain = self.simulate_chain(&ids, cell)?;
        let mut last = chain
            .pop()
            .ok_or_else(|| ApiError::new("internal", "nothing was simulated"))??;
        last.setup = setup;
        Ok(last)
    }

    /// Simulates setups in order, each starting from the stock the ones
    /// before it left (decision 0016, slice 4). Every setup machines along
    /// model Z, from the top or the bottom, so the stock is one column per
    /// cell from a low to a high Z; the grid is centred on the part, so
    /// turning it over maps cells exactly onto cells. A setup that cannot
    /// be simulated leaves the stock as it was.
    pub(crate) fn simulate_chain(
        &self,
        ids: &[u32],
        cell: Option<f64>,
    ) -> Result<Vec<Result<Simulated, ApiError>>, ApiError> {
        let cell = cell.unwrap_or(DEFAULT_CELL);
        if !(cell.is_finite() && (0.05..=5.0).contains(&cell)) {
            return Err(ApiError::new(
                "bad_request",
                "the cell size must be 0.05 to 5 mm",
            ));
        }
        // The carried stock: per cell of the top-side grid, its low and
        // high model Z. Made on the first setup that has a stock.
        let mut columns: Option<(cndt_sim::Grid, Vec<(f64, f64)>)> = None;
        let mut out = Vec::new();
        for id in ids {
            out.push(self.simulate_one(*id, cell, &mut columns));
        }
        Ok(out)
    }

    fn simulate_one(
        &self,
        id: u32,
        cell: f64,
        columns: &mut Option<(cndt_sim::Grid, Vec<(f64, f64)>)>,
    ) -> Result<Simulated, ApiError> {
        let document = self.session.document();
        let cam_error = |e: cndt_process_mill::CamError| ApiError::new("cam", e.to_string());
        let planned =
            cndt_process_mill::plan_setup(document, &self.rebuilt, self.kernel.as_ref(), Some(id))
                .map_err(cam_error)?;
        let Some(placed) = planned.setup else {
            return Err(ApiError::new("cam", "the setup has no placement"));
        };
        // A side setup is simulated on the part turned so that side is on
        // top (decision 0019).
        let world =
            cndt_process_mill::world(document, &self.rebuilt, self.kernel.as_ref(), placed.side)
                .map_err(cam_error)?;
        let stock = cndt_process_mill::stock_box(&world.document, &world.rebuilt, &placed)
            .map_err(cam_error)?;
        let body = world
            .rebuilt
            .body
            .as_ref()
            .ok_or_else(|| ApiError::new("no_body", "there is no solid yet"))?;
        // Centred on the stock: whole cells, the spare split evenly, so
        // turning the part over maps every cell centre onto another.
        let fit = |lo: f64, hi: f64| {
            let count = ((hi - lo) / cell).ceil().max(1.0);
            (lo - (count * cell - (hi - lo)) / 2.0, count as usize)
        };
        let (x0, nx) = fit(stock.min.x, stock.max.x);
        let (y0, ny) = fit(stock.min.y, stock.max.y);
        let grid = cndt_sim::Grid {
            x0,
            y0,
            cell,
            nx,
            ny,
        };
        // Which carried column each of this setup's cells is.
        let column_of = |c: usize| -> usize {
            let (i, j) = (c % nx, c / nx);
            match (placed.side, placed.flip_about) {
                (Side::Bottom, FlipAxis::X) => (ny - 1 - j) * nx + i,
                (Side::Bottom, FlipAxis::Y) => j * nx + (nx - 1 - i),
                _ => c,
            }
        };
        // Program Z from model Z and back: Z0 is on the face machined.
        let to_program = |z: f64| placed.program_point(Point3::new(0.0, 0.0, z)).z;
        let to_model = |z: f64| placed.model_point(Point3::new(0.0, 0.0, z)).z;
        let from_above = placed.side != Side::Bottom;
        // The carried columns run along model Z, so a side setup cannot use
        // them: it starts from the whole raw block, which can only show more
        // material than there is, never less, and leaves the columns as
        // they were.
        let lateral = placed.side.is_lateral();
        let mut fresh = lateral.then(|| (grid, vec![(stock.min.z, stock.max.z); grid.len()]));
        if !lateral && columns.as_ref().is_none_or(|(g, _)| *g != grid) {
            let (low, high) = if from_above {
                (to_model(stock.min.z), to_model(stock.max.z))
            } else {
                (to_model(stock.max.z), to_model(stock.min.z))
            };
            *columns = Some((grid, vec![(low, high); grid.len()]));
        }
        let Some((_, carried)) = (if lateral {
            fresh.as_mut()
        } else {
            columns.as_mut()
        }) else {
            return Err(ApiError::new("internal", "no stock to simulate"));
        };
        let (heights, floors): (Vec<f64>, Vec<f64>) = (0..grid.len())
            .map(|c| {
                let (low, high) = carried[column_of(c)];
                if lateral {
                    // Already in program Z.
                    (high, low)
                } else if from_above {
                    (to_program(high), to_program(low))
                } else {
                    (to_program(low), to_program(high))
                }
            })
            .unzip();
        // The part's top in each cell, seen along the spindle: lines
        // through the cell centres in model coordinates, back to program Z.
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
            .map(|(number, tool)| {
                let a = tool.assembly();
                let whole = cndt_sim::Tool {
                    cutter: shape(&tool.kind, tool.diameter),
                    flute_length: a.flute_length,
                    stickout: a.stickout,
                    shank_diameter: a.shank_diameter,
                    holder_diameter: a.holder_diameter,
                };
                (*number, whole)
            })
            .collect::<BTreeMap<_, _>>();
        let input = cndt_sim::Setup {
            grid,
            heights,
            floors,
            part,
        };
        let report = cndt_sim::simulate(&planned.program, &tools, &input)
            .map_err(|e| ApiError::new("simulation", e.to_string()))?;
        // What this setup left, back into the carried columns.
        for (c, h) in report.heights.iter().enumerate().filter(|_| !lateral) {
            let column = &mut carried[column_of(c)];
            let floor = input.floors[c];
            if *h <= floor {
                // Cut through: nothing left in this column.
                *column = (column.0, column.0);
            } else if from_above {
                column.1 = to_model(*h);
            } else {
                column.0 = to_model(*h);
            }
        }
        Ok(Simulated {
            revision: self.revision,
            setup: Some(id),
            report,
            part: input.part,
            floors: input.floors,
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
                fresh = self.simulate_setup(setup, Some(0.5), None)?;
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

    let class = |c: usize| -> u32 {
        match sim.part[c] {
            Some(top) if heights[c] < top - cndt_sim::TOLERANCE => 2,
            Some(top) if heights[c] > top + 0.05 => 1,
            Some(_) => 0,
            None => 3,
        }
    };
    let empty = |c: usize| sim.part[c].is_none() && heights[c] <= sim.floors[c] + 1e-9;
    let mut mesh = TriangleMesh {
        positions: Vec::with_capacity(heights.len() * 3),
        indices: Vec::new(),
        triangle_faces: Vec::new(),
        face_count: STOCK_CLASSES,
    };
    for ([x, y], h) in grid.centres().iter().zip(heights) {
        let m = sim.placed.in_model(Point3::new(*x, *y, *h));
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
