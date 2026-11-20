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
    /// What the simulation started from, so another program (the posted
    /// G-code read back) can be simulated on exactly the same stock.
    pub(crate) input: cndt_sim::Setup,
    /// The tools, by number.
    pub(crate) tools: BTreeMap<u32, cndt_sim::Tool>,
}

/// The stock carried from one setup to the next.
pub(crate) enum Carry {
    /// One column per cell along model Z: top and bottom setups only.
    Columns(Option<(cndt_sim::Grid, Vec<(f64, f64)>)>),
    /// Columns along all three model axes: when any setup is on a side.
    Dexels(Option<cndt_sim::Dexels>),
}

/// How a setup's program axes lie in the model.
fn placement_of(placed: &cndt_process_mill::Setup) -> Option<cndt_sim::Placement> {
    let at = |x: f64, y: f64, z: f64| {
        let m = placed.in_model(Point3::new(x, y, z));
        [m.x, m.y, m.z]
    };
    let origin = at(0.0, 0.0, 0.0);
    let direction = |m: [f64; 3]| [m[0] - origin[0], m[1] - origin[1], m[2] - origin[2]];
    cndt_sim::Placement::new(
        origin,
        [
            direction(at(1.0, 0.0, 0.0)),
            direction(at(0.0, 1.0, 0.0)),
            direction(at(0.0, 0.0, 1.0)),
        ],
    )
}

/// A simulation failure as an API error: a cancel has its own kind.
pub(crate) fn sim_error(error: cndt_sim::SimError) -> ApiError {
    let kind = if error == cndt_sim::SimError::Cancelled {
        "cancelled"
    } else {
        "simulation"
    };
    ApiError::new(kind, error.to_string())
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
        // Top and bottom only: one column per cell along model Z, exact.
        // With a side among them: columns along all three axes (decision
        // 0019).
        let sides = &self.session.document().cam().setups;
        let lateral = ids.iter().any(|id| {
            sides
                .iter()
                .any(|s| s.id == *id && s.setup.side.is_lateral())
        });
        let mut carry = if lateral {
            Carry::Dexels(None)
        } else {
            Carry::Columns(None)
        };
        let mut out = Vec::new();
        for id in ids {
            out.push(self.simulate_one(*id, cell, &mut carry));
        }
        Ok(out)
    }

    fn simulate_one(&self, id: u32, cell: f64, carry: &mut Carry) -> Result<Simulated, ApiError> {
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
            cndt_process_mill::world(document, &self.rebuilt, self.kernel.as_ref(), placed.turn)
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
        // With a vise anywhere, every setup's grid reaches past the stock
        // by the thickest jaw, the same on every side, so carried cells
        // still map onto cells and the jaws are on the grid.
        let margin = document
            .cam()
            .setups
            .iter()
            .filter_map(|s| s.setup.vise.map(|v| v.jaw))
            .fold(0.0, f64::max);
        let (x0, nx) = fit(stock.min.x - margin, stock.max.x + margin);
        let (y0, ny) = fit(stock.min.y - margin, stock.max.y + margin);
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
        // A setup at an angle that is not a quarter turn (decision 0032)
        // has no placement on the carried columns: it starts from the box
        // round the turned raw stock, and leaves the carried stock as it
        // was. It can show more material than there is, never less.
        let placement = placement_of(&placed);
        // The real stock in program coordinates (a side setup's planning
        // box is larger, conservatively): where the jaws grip it, and where
        // the grid's margin holds no stock.
        let exact = if let Some(placement) = &placement {
            let (min, max) = self.model_stock()?;
            let corners = (0..8).map(|k| {
                placement.program([
                    if k & 1 == 0 { min[0] } else { max[0] },
                    if k & 2 == 0 { min[1] } else { max[1] },
                    if k & 4 == 0 { min[2] } else { max[2] },
                ])
            });
            corners.fold(
                ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
                |(lo, hi), p| {
                    (
                        [0, 1, 2].map(|a| lo[a].min(p[a])),
                        [0, 1, 2].map(|a| hi[a].max(p[a])),
                    )
                },
            )
        } else {
            (
                [stock.min.x, stock.min.y, stock.min.z],
                [stock.max.x, stock.max.y, stock.max.z],
            )
        };
        let on_stock = |c: usize| {
            let [x, y] = grid.centres_at(c);
            (exact.0[0]..=exact.1[0]).contains(&x) && (exact.0[1]..=exact.1[1]).contains(&y)
        };
        let fixture: Vec<Option<f64>> = match document
            .cam()
            .setups
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.setup.vise)
        {
            None => Vec::new(),
            Some(vise) => {
                let top = exact.0[2] + vise.grip;
                // Across the jaws' axis: the jaws stand outside the stock's
                // two faces, `jaw` thick, along the whole grid.
                let across = match vise.along {
                    cndt_model::cam::JawAxis::X => 1,
                    cndt_model::cam::JawAxis::Y => 0,
                };
                let (lo, hi) = (exact.0[across], exact.1[across]);
                let parallel = cndt_model::cam::PARALLEL;
                (0..grid.len())
                    .map(|c| {
                        let v = grid.centres_at(c)[across];
                        if (lo - vise.jaw..lo).contains(&v) || (hi..hi + vise.jaw).contains(&v) {
                            Some(top)
                        } else if (lo..lo + parallel).contains(&v)
                            || (hi - parallel..hi).contains(&v)
                        {
                            // The parallels under the clamped faces: their
                            // tops at the stock's bottom.
                            Some(exact.0[2])
                        } else if (lo..hi).contains(&v) {
                            // Between the parallels: open down to the
                            // vise's floor, when it has one.
                            vise.body.map(|body| exact.0[2] - body)
                        } else {
                            None
                        }
                    })
                    .collect()
            }
        };
        let (heights, floors): (Vec<f64>, Vec<f64>) = match (&placement, &mut *carry) {
            (None, _) => (0..grid.len())
                .map(|c| {
                    if on_stock(c) {
                        (stock.max.z, stock.min.z)
                    } else {
                        (stock.min.z, stock.min.z)
                    }
                })
                .unzip(),
            (Some(placement), Carry::Dexels(dexels)) => {
                if dexels.is_none() {
                    let (min, max) = self.model_stock()?;
                    *dexels = Some(cndt_sim::Dexels::block(min, max, cell));
                }
                let Some(dexels) = dexels.as_ref() else {
                    return Err(ApiError::new("internal", "no stock to simulate"));
                };
                dexels.height_map(placement, &grid, stock.min.z)
            }
            (Some(_), Carry::Columns(columns)) => {
                if columns.as_ref().is_none_or(|(g, _)| *g != grid) {
                    let (low, high) = if from_above {
                        (to_model(stock.min.z), to_model(stock.max.z))
                    } else {
                        (to_model(stock.max.z), to_model(stock.min.z))
                    };
                    // Cells in the margin past the stock hold nothing.
                    *columns = Some((
                        grid,
                        (0..grid.len())
                            .map(|c| if on_stock(c) { (low, high) } else { (low, low) })
                            .collect(),
                    ));
                }
                let Some((_, carried)) = columns.as_ref() else {
                    return Err(ApiError::new("internal", "no stock to simulate"));
                };
                (0..grid.len())
                    .map(|c| {
                        let (low, high) = carried[column_of(c)];
                        if from_above {
                            (to_program(high), to_program(low))
                        } else {
                            (to_program(low), to_program(high))
                        }
                    })
                    .unzip()
            }
        };
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
            fixture,
        };
        let report = cndt_sim::simulate_controlled(&planned.program, &tools, &input, &self.control)
            .map_err(sim_error)?;
        // What this setup left, back into the carried stock.
        match (&placement, carry) {
            (None, _) | (Some(_), Carry::Dexels(None)) => {}
            (Some(placement), Carry::Dexels(Some(dexels))) => {
                dexels.cut(placement, &grid, &report.heights);
            }
            (Some(_), Carry::Columns(columns)) => {
                if let Some((_, carried)) = columns.as_mut() {
                    for (c, h) in report.heights.iter().enumerate() {
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
                }
            }
        }
        Ok(Simulated {
            revision: self.revision,
            setup: Some(id),
            report,
            part: input.part.clone(),
            floors: input.floors.clone(),
            placed,
            input,
            tools,
        })
    }

    /// The raw stock in model coordinates: the part's box with the stock
    /// allowances round it.
    fn model_stock(&self) -> Result<([f64; 3], [f64; 3]), ApiError> {
        let stock = self
            .session
            .document()
            .cam()
            .stock
            .ok_or_else(|| ApiError::new("cam", "the stock is not set"))?;
        let bounds = self
            .rebuilt
            .properties
            .ok_or_else(|| ApiError::new("no_body", "there is no solid yet"))?
            .bounds;
        Ok((
            [
                bounds.min.x - stock.side,
                bounds.min.y - stock.side,
                bounds.min.z - stock.bottom,
            ],
            [
                bounds.max.x + stock.side,
                bounds.max.y + stock.side,
                bounds.max.z + stock.top,
            ],
        ))
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
