// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 3 stock simulation (decision 0016).
//!
//! One setup at a time, the stock is a height map in program coordinates:
//! a grid of cells seen along the spindle, each holding the height of the
//! stock's top there. Every move sweeps the tool's shape through it. The
//! simulation finds what can be found that way, without cutting anything
//! real:
//!
//! - **collisions:** a rapid move that would remove material, which on a
//!   machine is the tool crashing into the stock;
//! - **gouges:** a cut that goes below the finished part, compared with
//!   the part's exact surface, not a mesh of it;
//! - what is **left** on the part, the lengths and the cutting time.
//!
//! A height map sees only what faces the spindle, which is all a 3-axis
//! setup can reach. It knows nothing of tool holders or of what other
//! setups removed; those come later.

use std::collections::BTreeMap;
use std::fmt;

use cndt_cam_core::{ArcDirection, Event, Move, Program, Step};
use cndt_geom::Point3;
use serde::Serialize;

/// How far below the stock a rapid may reach, or below the part a cut,
/// before it counts, in millimetres.
pub const TOLERANCE: f64 = 0.01;

/// The cutting shape of a tool, from its tip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolShape {
    /// Flat bottomed: end mills, face mills, reamers.
    Flat {
        /// Diameter in millimetres.
        diameter: f64,
    },
    /// Pointed: drills (118°), spot drills and chamfer mills (90°). A cone
    /// up to the full diameter, then straight.
    Cone {
        /// Diameter in millimetres.
        diameter: f64,
        /// The included angle of the point, in degrees.
        angle: f64,
    },
}

impl ToolShape {
    fn radius(self) -> f64 {
        match self {
            Self::Flat { diameter } | Self::Cone { diameter, .. } => diameter / 2.0,
        }
    }

    /// How far above the tip the tool's surface is at distance `d` from its
    /// axis.
    fn lift(self, d: f64) -> f64 {
        match self {
            Self::Flat { .. } => 0.0,
            Self::Cone { angle, .. } => d / (angle.to_radians() / 2.0).tan(),
        }
    }

    /// How far apart to sample a move. Between samples a flat tool leaves
    /// scallops of at most 0.005 mm; a cone is sampled at half a cell.
    fn spacing(self, cell: f64) -> f64 {
        match self {
            Self::Flat { diameter } => (4.0 * diameter * 0.005).sqrt().max(cell / 2.0),
            Self::Cone { .. } => cell / 2.0,
        }
    }

    fn check(self) -> bool {
        match self {
            Self::Flat { diameter } => diameter.is_finite() && diameter > 0.0,
            Self::Cone { diameter, angle } => {
                diameter.is_finite() && diameter > 0.0 && angle > 0.0 && angle < 180.0
            }
        }
    }
}

/// A whole tool as it sits in its holder: the cutting shape at the tip,
/// the plain shank above the flutes, and the holder above that. Only the
/// cutting part may touch the stock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tool {
    /// The cutting shape.
    pub cutter: ToolShape,
    /// How far up from the tip it cuts, in mm.
    pub flute_length: f64,
    /// How far up from the tip the holder's face is, in mm.
    pub stickout: f64,
    /// The plain shank's diameter, in mm.
    pub shank_diameter: f64,
    /// The holder's diameter, in mm.
    pub holder_diameter: f64,
}

impl Tool {
    /// A cutter with nothing above it in the way: for tests and for tools
    /// whose assembly does not matter.
    #[must_use]
    pub fn bare(cutter: ToolShape) -> Self {
        Self {
            cutter,
            flute_length: 1000.0,
            stickout: 1000.0,
            shank_diameter: cutter.radius() * 2.0,
            holder_diameter: cutter.radius() * 2.0,
        }
    }

    fn check(self) -> bool {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        self.cutter.check()
            && positive(self.flute_length)
            && positive(self.stickout)
            && positive(self.shank_diameter)
            && positive(self.holder_diameter)
            && self.stickout >= self.flute_length
    }
}

/// What a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Hit {
    /// A rapid move removed material: the tool crashed into the stock.
    Rapid,
    /// The plain shank above the flutes touched the stock: the cut is
    /// deeper than the flutes are long.
    Shank,
    /// The holder touched the stock: the tool sticks out too little.
    Holder,
    /// A cut went below the finished part.
    Gouge,
}

/// A grid of square cells in the setup's XY plane. Cell `(i, j)` has its
/// centre at `(x0 + (i + ½)·cell, y0 + (j + ½)·cell)` and index `j·nx + i`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Grid {
    /// The left edge.
    pub x0: f64,
    /// The front edge.
    pub y0: f64,
    /// The cell size in millimetres.
    pub cell: f64,
    /// Cells along X.
    pub nx: usize,
    /// Cells along Y.
    pub ny: usize,
}

impl Grid {
    /// The grid over a rectangle, with cells of about `cell` millimetres.
    #[must_use]
    pub fn over(x0: f64, y0: f64, x1: f64, y1: f64, cell: f64) -> Self {
        let count = |length: f64| ((length / cell).ceil().max(1.0)) as usize;
        Self {
            x0,
            y0,
            cell,
            nx: count(x1 - x0),
            ny: count(y1 - y0),
        }
    }

    /// Every cell's centre, in index order.
    #[must_use]
    pub fn centres(&self) -> Vec<[f64; 2]> {
        let mut out = Vec::with_capacity(self.nx * self.ny);
        for j in 0..self.ny {
            for i in 0..self.nx {
                out.push([
                    self.x0 + (i as f64 + 0.5) * self.cell,
                    self.y0 + (j as f64 + 0.5) * self.cell,
                ]);
            }
        }
        out
    }

    fn len(&self) -> usize {
        self.nx * self.ny
    }
}

/// The stock and the part for one setup, in program coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// The cells.
    pub grid: Grid,
    /// The stock's top before cutting.
    pub top: f64,
    /// The stock's bottom: nothing is removed below it.
    pub bottom: f64,
    /// The finished part's top in each cell, seen along the spindle;
    /// `None` where there is no part.
    pub part: Vec<Option<f64>>,
}

/// Something wrong, where it happened.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// What it is.
    pub what: Hit,
    /// The operation's name.
    pub operation: String,
    /// The step in it, counting from 1.
    pub step: usize,
    /// The deepest point, in program coordinates.
    pub at: Point3,
    /// How deep into the stock (a collision) or the part (a gouge), in mm.
    pub depth: f64,
}

/// What the simulation found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// Rapid moves into material, and shanks or holders touching the
    /// stock: the deepest of each kind on each move.
    pub collisions: Vec<Finding>,
    /// Cuts below the finished part, the deepest of each operation.
    pub gouges: Vec<Finding>,
    /// The most material left on the part, in mm.
    pub left: f64,
    /// Where that is: the top of the material left, in program
    /// coordinates. `None` when nothing is left.
    pub left_at: Option<Point3>,
    /// Material removed, in mm³.
    pub removed: f64,
    /// Length of cutting moves, in mm.
    pub cut_length: f64,
    /// Length of rapid moves, in mm.
    pub rapid_length: f64,
    /// Time spent cutting at the programmed feeds, in minutes. Rapids, tool
    /// changes and dwells are not counted.
    pub cut_minutes: f64,
    /// The cells.
    pub grid: Grid,
    /// The stock's top in each cell afterwards.
    pub heights: Vec<f64>,
}

/// Why a program could not be simulated.
#[derive(Debug, Clone, PartialEq)]
pub enum SimError {
    /// The program uses a tool with no shape.
    MissingTool(u32),
    /// The grid, a height or a tool is unusable; says which.
    BadInput(String),
    /// The program turns the table or changes the offset after cutting:
    /// simulate one side at a time.
    SeveralSides,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTool(number) => write!(f, "tool {number} has no shape to simulate"),
            Self::BadInput(what) => write!(f, "cannot simulate: {what}"),
            Self::SeveralSides => f.write_str(
                "the program machines more than one side; simulate each setup on its own",
            ),
        }
    }
}

impl std::error::Error for SimError {}

/// The most cells a simulation may have.
const MAX_CELLS: usize = 16_000_000;

/// Simulates a program on one setup's stock.
///
/// # Errors
///
/// [`SimError`] for a tool without a shape, unusable input, or a program
/// that turns to another side after cutting.
pub fn simulate(
    program: &Program,
    tools: &BTreeMap<u32, Tool>,
    setup: &Setup,
) -> Result<Report, SimError> {
    let grid = setup.grid;
    let bad = |what: &str| Err(SimError::BadInput(what.to_owned()));
    if !(grid.cell.is_finite() && grid.cell > 0.0 && grid.x0.is_finite() && grid.y0.is_finite()) {
        return bad("the grid's cells must be more than zero");
    }
    if grid.len() == 0 || grid.len() > MAX_CELLS {
        return bad("the grid has no cells or too many");
    }
    if setup.part.len() != grid.len() {
        return bad("the part heights do not match the grid");
    }
    if !(setup.top.is_finite() && setup.bottom.is_finite() && setup.bottom < setup.top) {
        return bad("the stock's bottom must be below its top");
    }
    if let Some((number, _)) = tools.iter().find(|(_, tool)| !tool.check()) {
        return bad(&format!("tool {number}'s shape or assembly"));
    }
    let mut sim = Sim {
        setup,
        heights: vec![setup.top; grid.len()],
        highest: setup.top,
        tool: None,
        position: None,
        moved: false,
        removed: 0.0,
        collisions: Vec::new(),
        gouges: BTreeMap::new(),
        cut_length: 0.0,
        rapid_length: 0.0,
        cut_minutes: 0.0,
    };
    for (index, operation) in program.operations.iter().enumerate() {
        for (number, step) in operation.steps.iter().enumerate() {
            let at = At {
                operation: index,
                name: &operation.name,
                step: number + 1,
            };
            sim.step(step, tools, &at)?;
        }
    }
    let centres = grid.centres();
    let mut left = 0.0_f64;
    let mut left_at = None;
    for ((h, part), [x, y]) in sim.heights.iter().zip(&setup.part).zip(&centres) {
        if let Some(p) = part
            && h - p > left
        {
            left = h - p;
            left_at = Some(Point3::new(*x, *y, *h));
        }
    }
    Ok(Report {
        collisions: sim.collisions,
        gouges: sim.gouges.into_values().collect(),
        left,
        left_at,
        removed: sim.removed,
        cut_length: sim.cut_length,
        rapid_length: sim.rapid_length,
        cut_minutes: sim.cut_minutes,
        grid,
        heights: sim.heights,
    })
}

/// Where in the program a step is.
struct At<'a> {
    operation: usize,
    name: &'a str,
    step: usize,
}

impl At<'_> {
    fn finding(&self, what: Hit, at: Point3, depth: f64) -> Finding {
        Finding {
            what,
            operation: self.name.to_owned(),
            step: self.step,
            at,
            depth,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rapid,
    Cut,
}

struct Sim<'a> {
    setup: &'a Setup,
    heights: Vec<f64>,
    /// No stock is higher than this: the shank and holder need checking
    /// only when they reach below it.
    highest: f64,
    tool: Option<Tool>,
    position: Option<Point3>,
    moved: bool,
    removed: f64,
    collisions: Vec<Finding>,
    /// The deepest gouge of each operation, by its index.
    gouges: BTreeMap<usize, Finding>,
    cut_length: f64,
    rapid_length: f64,
    cut_minutes: f64,
}

impl Sim<'_> {
    fn step(
        &mut self,
        step: &Step,
        tools: &BTreeMap<u32, Tool>,
        at: &At<'_>,
    ) -> Result<(), SimError> {
        match step {
            Step::Event(Event::ToolChange { number, .. }) => {
                self.tool = Some(*tools.get(number).ok_or(SimError::MissingTool(*number))?);
                self.position = None;
                self.highest = self.heights.iter().copied().fold(f64::MIN, f64::max);
            }
            Step::Event(Event::Home) => self.position = None,
            Step::Event(Event::Index { .. } | Event::WorkOffset { .. }) => {
                if self.moved {
                    return Err(SimError::SeveralSides);
                }
                self.position = None;
            }
            Step::Event(_) | Step::Move(Move::Dwell { .. }) => {}
            Step::Move(Move::Rapid { to }) => self.line(*to, Kind::Rapid, None, at),
            Step::Move(Move::Linear { to, feed }) => self.line(*to, Kind::Cut, Some(*feed), at),
            Step::Move(Move::Arc {
                to,
                center,
                direction,
                feed,
            }) => self.arc(*to, *center, *direction, *feed, at),
            Step::Move(Move::Cycle(cycle)) => {
                let start = self.position.map_or(cycle.clearance, |p| p.z);
                for hole in &cycle.holes {
                    self.line(Point3::new(hole.x, hole.y, start), Kind::Rapid, None, at);
                    self.line(
                        Point3::new(hole.x, hole.y, cycle.clearance),
                        Kind::Rapid,
                        None,
                        at,
                    );
                    self.line(
                        Point3::new(hole.x, hole.y, cycle.bottom),
                        Kind::Cut,
                        Some(cycle.feed),
                        at,
                    );
                    // Out the way it went in: nothing more to remove.
                    self.position = Some(Point3::new(hole.x, hole.y, start));
                    self.rapid_length += start - cycle.bottom;
                }
            }
        }
        Ok(())
    }

    /// A straight move from the current position.
    fn line(&mut self, to: Point3, kind: Kind, feed: Option<f64>, at: &At<'_>) {
        self.moved = true;
        let Some(from) = self.position.replace(to) else {
            // Where the tool was is unknown: only where it arrives counts.
            self.sweep(&[to], kind, at);
            return;
        };
        let length = distance(from, to);
        self.count(length, kind, feed);
        let Some(tool) = self.tool else { return };
        let n = (length / tool.cutter.spacing(self.setup.grid.cell))
            .ceil()
            .max(1.0) as usize;
        let points: Vec<Point3> = (0..=n)
            .map(|k| {
                let t = k as f64 / n as f64;
                Point3::new(
                    from.x + (to.x - from.x) * t,
                    from.y + (to.y - from.y) * t,
                    from.z + (to.z - from.z) * t,
                )
            })
            .collect();
        self.sweep(&points, kind, at);
    }

    /// An arc in XY from the current position, Z changing evenly.
    fn arc(&mut self, to: Point3, center: Point3, direction: ArcDirection, feed: f64, at: &At<'_>) {
        self.moved = true;
        let Some(from) = self.position.replace(to) else {
            self.sweep(&[to], Kind::Cut, at);
            return;
        };
        let radius = (from.x - center.x).hypot(from.y - center.y);
        let a0 = (from.y - center.y).atan2(from.x - center.x);
        let a1 = (to.y - center.y).atan2(to.x - center.x);
        let tau = std::f64::consts::TAU;
        let mut sweep = match direction {
            ArcDirection::CounterClockwise => (a1 - a0).rem_euclid(tau),
            ArcDirection::Clockwise => -(a0 - a1).rem_euclid(tau),
        };
        if sweep.abs() < 1e-9 {
            // Same start and end: a full circle.
            sweep = match direction {
                ArcDirection::CounterClockwise => tau,
                ArcDirection::Clockwise => -tau,
            };
        }
        let length = (radius * sweep.abs()).hypot(to.z - from.z);
        self.count(length, Kind::Cut, Some(feed));
        let Some(tool) = self.tool else { return };
        let n = (length / tool.cutter.spacing(self.setup.grid.cell))
            .ceil()
            .max(1.0) as usize;
        let points: Vec<Point3> = (0..=n)
            .map(|k| {
                let t = k as f64 / n as f64;
                let a = a0 + sweep * t;
                Point3::new(
                    center.x + radius * a.cos(),
                    center.y + radius * a.sin(),
                    from.z + (to.z - from.z) * t,
                )
            })
            .collect();
        self.sweep(&points, Kind::Cut, at);
    }

    fn count(&mut self, length: f64, kind: Kind, feed: Option<f64>) {
        match kind {
            Kind::Rapid => self.rapid_length += length,
            Kind::Cut => {
                self.cut_length += length;
                if let Some(feed) = feed.filter(|f| *f > 0.0) {
                    self.cut_minutes += length / feed;
                }
            }
        }
    }

    /// Puts the tool's tip at each point in turn, lowering the stock under
    /// it, and records what that hits.
    fn sweep(&mut self, points: &[Point3], kind: Kind, at: &At<'_>) {
        let Some(whole) = self.tool else { return };
        let tool = whole.cutter;
        let grid = self.setup.grid;
        let r = tool.radius();
        // The worst touch of the shank and of the holder on this move.
        let mut touched: [Option<(Point3, f64)>; 2] = [None, None];
        let area = grid.cell * grid.cell;
        // The deepest removal on this move (a rapid) or below the part (a cut).
        let mut worst: Option<(Point3, f64)> = None;
        // For a rapid, each cell's height before the move: the samples
        // before the last have already lowered it.
        let mut before_move: BTreeMap<usize, f64> = BTreeMap::new();
        for p in points {
            // Before this point cuts: whatever stands above the flutes, or
            // under the holder, is hit by a part that cannot cut.
            self.check_assembly(whole, *p, &mut touched);
            let range = |lo: f64, hi: f64, origin: f64, count: usize| {
                let first = ((lo - origin) / grid.cell).floor().max(0.0) as usize;
                let last = (((hi - origin) / grid.cell).ceil().max(0.0) as usize).min(count);
                first..last
            };
            for j in range(p.y - r, p.y + r, grid.y0, grid.ny) {
                let cy = grid.y0 + (j as f64 + 0.5) * grid.cell;
                for i in range(p.x - r, p.x + r, grid.x0, grid.nx) {
                    let cx = grid.x0 + (i as f64 + 0.5) * grid.cell;
                    let d = (cx - p.x).hypot(cy - p.y);
                    if d > r {
                        continue;
                    }
                    let c = j * grid.nx + i;
                    let z = (p.z + tool.lift(d)).max(self.setup.bottom);
                    let before = self.heights[c];
                    if z >= before {
                        continue;
                    }
                    self.heights[c] = z;
                    self.removed += (before - z) * area;
                    let depth = match kind {
                        Kind::Rapid => *before_move.entry(c).or_insert(before) - z,
                        Kind::Cut => self.setup.part[c].map_or(0.0, |top| top - z),
                    };
                    if depth > TOLERANCE && worst.is_none_or(|(_, d)| depth > d) {
                        worst = Some((Point3::new(cx, cy, z), depth));
                    }
                }
            }
        }
        for (hit, touch) in [Hit::Shank, Hit::Holder].into_iter().zip(touched) {
            if let Some((point, depth)) = touch {
                self.collisions.push(at.finding(hit, point, depth));
            }
        }
        let Some((point, depth)) = worst else { return };
        match kind {
            Kind::Rapid => self.collisions.push(at.finding(Hit::Rapid, point, depth)),
            Kind::Cut => {
                let deeper = self
                    .gouges
                    .get(&at.operation)
                    .is_none_or(|known| depth > known.depth);
                if deeper {
                    self.gouges
                        .insert(at.operation, at.finding(Hit::Gouge, point, depth));
                }
            }
        }
    }

    /// Whether the shank or holder, with the tip at `p`, touches stock
    /// that is still there; keeps the deepest touch of each.
    fn check_assembly(&self, whole: Tool, p: Point3, touched: &mut [Option<(Point3, f64)>; 2]) {
        let grid = self.setup.grid;
        let parts = [
            // Above the flutes, nothing cuts across the whole cutter's width.
            (
                whole.shank_diameter.max(whole.cutter.radius() * 2.0) / 2.0,
                p.z + whole.flute_length,
            ),
            (whole.holder_diameter / 2.0, p.z + whole.stickout),
        ];
        for ((radius, base), touch) in parts.into_iter().zip(touched.iter_mut()) {
            if base + TOLERANCE >= self.highest {
                continue;
            }
            let span = |lo: f64, hi: f64, origin: f64, count: usize| {
                let first = ((lo - origin) / grid.cell).floor().max(0.0) as usize;
                let last = (((hi - origin) / grid.cell).ceil().max(0.0) as usize).min(count);
                first..last
            };
            for j in span(p.y - radius, p.y + radius, grid.y0, grid.ny) {
                let cy = grid.y0 + (j as f64 + 0.5) * grid.cell;
                for i in span(p.x - radius, p.x + radius, grid.x0, grid.nx) {
                    let cx = grid.x0 + (i as f64 + 0.5) * grid.cell;
                    if (cx - p.x).hypot(cy - p.y) > radius {
                        continue;
                    }
                    let depth = self.heights[j * grid.nx + i] - base;
                    if depth > TOLERANCE && touch.is_none_or(|(_, d)| depth > d) {
                        *touch = Some((Point3::new(cx, cy, base), depth));
                    }
                }
            }
        }
    }
}

fn distance(a: Point3, b: Point3) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use cndt_cam_core::{Cycle, CycleKind, Operation, SpindleDirection};

    fn p(x: f64, y: f64, z: f64) -> Point3 {
        Point3::new(x, y, z)
    }

    fn rapid(x: f64, y: f64, z: f64) -> Step {
        Step::Move(Move::Rapid { to: p(x, y, z) })
    }

    fn cut(x: f64, y: f64, z: f64) -> Step {
        Step::Move(Move::Linear {
            to: p(x, y, z),
            feed: 600.0,
        })
    }

    fn program(steps: Vec<Step>) -> Program {
        let mut all = vec![
            Step::Event(Event::ToolChange {
                number: 1,
                description: String::new(),
            }),
            Step::Event(Event::SpindleOn {
                rpm: 1000.0,
                direction: SpindleDirection::Clockwise,
            }),
        ];
        all.extend(steps);
        Program {
            name: String::from("test"),
            number: 1,
            work_offset: 1,
            operations: vec![Operation {
                name: String::from("Op"),
                steps: all,
            }],
        }
    }

    /// A 20 x 20 stock, 0 to −10 deep, with a part whose top is at −1
    /// everywhere except a 10 x 10 pocket to −5 in the middle.
    fn block() -> Setup {
        let grid = Grid::over(0.0, 0.0, 20.0, 20.0, 0.25);
        let part = grid
            .centres()
            .iter()
            .map(|[x, y]| {
                let inside = (5.0..15.0).contains(x) && (5.0..15.0).contains(y);
                Some(if inside { -5.0 } else { -1.0 })
            })
            .collect();
        Setup {
            grid,
            top: 0.0,
            bottom: -10.0,
            part,
        }
    }

    fn tools(shape: ToolShape) -> BTreeMap<u32, Tool> {
        BTreeMap::from([(1, Tool::bare(shape))])
    }

    #[test]
    fn a_rapid_into_the_stock_is_a_collision() {
        let setup = block();
        let report = simulate(
            &program(vec![rapid(10.0, 10.0, 5.0), rapid(10.0, 10.0, -0.5)]),
            &tools(ToolShape::Flat { diameter: 4.0 }),
            &setup,
        )
        .unwrap();
        assert_eq!(report.collisions.len(), 1, "{report:?}");
        let hit = &report.collisions[0];
        assert_eq!(hit.step, 4);
        assert!((hit.depth - 0.5).abs() < 1e-9, "{hit:?}");
        // Above the stock: nothing.
        let report = simulate(
            &program(vec![rapid(10.0, 10.0, 5.0), rapid(30.0, 10.0, 0.5)]),
            &tools(ToolShape::Flat { diameter: 4.0 }),
            &setup,
        )
        .unwrap();
        assert!(report.collisions.is_empty());
        assert!(report.removed.abs() < 1e-12);
    }

    #[test]
    fn cutting_the_pocket_to_its_floor_is_clean_and_deeper_is_a_gouge() {
        let setup = block();
        // A 4 mm end mill round the pocket's wall, 2 mm in from it, at −5.
        let path = |z: f64| {
            vec![
                rapid(7.0, 7.0, 2.0),
                cut(7.0, 7.0, z),
                cut(13.0, 7.0, z),
                cut(13.0, 13.0, z),
                cut(7.0, 13.0, z),
                cut(7.0, 7.0, z),
                rapid(7.0, 7.0, 2.0),
            ]
        };
        let report = simulate(
            &program(path(-5.0)),
            &tools(ToolShape::Flat { diameter: 4.0 }),
            &setup,
        )
        .unwrap();
        assert!(report.gouges.is_empty(), "{report:?}");
        assert!(report.collisions.is_empty(), "{report:?}");
        // The walls are reached exactly, the middle is left at the top.
        assert!((report.left - 5.0).abs() < 1e-9, "{}", report.left);
        // 7 mm down from the rapid, then four 6 mm sides.
        assert!((report.cut_length - 31.0).abs() < 1e-9);
        assert!((report.cut_minutes - 31.0 / 600.0).abs() < 1e-12);
        // Half a millimetre too deep.
        let report = simulate(
            &program(path(-5.5)),
            &tools(ToolShape::Flat { diameter: 4.0 }),
            &setup,
        )
        .unwrap();
        assert_eq!(report.gouges.len(), 1);
        assert!((report.gouges[0].depth - 0.5).abs() < 1e-9);
        // A tool too big for the path cuts into the walls.
        let report = simulate(
            &program(path(-5.0)),
            &tools(ToolShape::Flat { diameter: 4.6 }),
            &setup,
        )
        .unwrap();
        assert_eq!(report.gouges.len(), 1);
        assert!((report.gouges[0].depth - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_drill_point_and_an_arc() {
        let setup = block();
        // A 118° drill to −3 in the pocket's middle. No cell centre is on
        // its axis: the nearest are 0.177 mm off, where the point is 0.106
        // mm up (d / tan 59°).
        let drill = Step::Move(Move::Cycle(Cycle {
            kind: CycleKind::Drill,
            holes: vec![p(10.0, 10.0, 0.0)],
            clearance: 2.0,
            bottom: -3.0,
            feed: 100.0,
        }));
        let report = simulate(
            &program(vec![rapid(10.0, 10.0, 5.0), drill]),
            &tools(ToolShape::Cone {
                diameter: 2.0,
                angle: 118.0,
            }),
            &setup,
        )
        .unwrap();
        assert!(report.collisions.is_empty() && report.gouges.is_empty());
        let lowest = report.heights.iter().copied().fold(f64::MAX, f64::min);
        assert!(
            (lowest - (-3.0 + 0.125_f64.hypot(0.125) / 59_f64.to_radians().tan())).abs() < 1e-9
        );
        // A full circle at −5 of radius 3 round the middle: inside the pocket.
        let circle = Step::Move(Move::Arc {
            to: p(13.0, 10.0, -5.0),
            center: p(10.0, 10.0, -5.0),
            direction: ArcDirection::CounterClockwise,
            feed: 300.0,
        });
        let report = simulate(
            &program(vec![rapid(13.0, 10.0, 2.0), cut(13.0, 10.0, -5.0), circle]),
            &tools(ToolShape::Flat { diameter: 4.0 }),
            &setup,
        )
        .unwrap();
        assert!(report.gouges.is_empty(), "{report:?}");
        let expected = 7.0 + std::f64::consts::TAU * 3.0;
        assert!((report.cut_length - expected).abs() < 1e-9);
        // Radius 3.5 with a 4 mm tool reaches 5.5 from the middle: a gouge.
        let wide = Step::Move(Move::Arc {
            to: p(13.5, 10.0, -5.0),
            center: p(10.0, 10.0, -5.0),
            direction: ArcDirection::Clockwise,
            feed: 300.0,
        });
        let report = simulate(
            &program(vec![rapid(13.5, 10.0, 2.0), cut(13.5, 10.0, -5.0), wide]),
            &tools(ToolShape::Flat { diameter: 4.0 }),
            &setup,
        )
        .unwrap();
        assert_eq!(report.gouges.len(), 1);
    }

    #[test]
    fn bad_input_and_several_sides() {
        let setup = block();
        let error = simulate(&program(vec![]), &BTreeMap::new(), &setup).unwrap_err();
        assert_eq!(error, SimError::MissingTool(1));
        let mut wrong = setup.clone();
        wrong.part.pop();
        assert!(matches!(
            simulate(
                &program(vec![]),
                &tools(ToolShape::Flat { diameter: 1.0 }),
                &wrong
            ),
            Err(SimError::BadInput(_))
        ));
        let two = program(vec![
            rapid(1.0, 1.0, 5.0),
            Step::Event(Event::WorkOffset { offset: 2 }),
        ]);
        assert_eq!(
            simulate(&two, &tools(ToolShape::Flat { diameter: 1.0 }), &setup),
            Err(SimError::SeveralSides)
        );
    }

    #[test]
    fn the_shank_and_holder_must_not_touch_the_stock() {
        let setup = block();
        let mill = |flute: f64, stickout: f64| {
            BTreeMap::from([(
                1,
                Tool {
                    cutter: ToolShape::Flat { diameter: 4.0 },
                    flute_length: flute,
                    stickout,
                    shank_diameter: 4.0,
                    holder_diameter: 20.0,
                },
            )])
        };
        // Into the stock from the side at −5: 5 mm of wall.
        let side = program(vec![
            rapid(-5.0, 10.0, 2.0),
            rapid(-5.0, 10.0, -5.0),
            cut(10.0, 10.0, -5.0),
            rapid(10.0, 10.0, 2.0),
        ]);
        // Long enough flutes, the holder well clear: clean.
        let report = simulate(&side, &mill(8.0, 30.0), &setup).unwrap();
        assert!(report.collisions.is_empty(), "{report:?}");
        // 3 mm flutes: the shank rubs the top 2 mm of the wall.
        let report = simulate(&side, &mill(3.0, 30.0), &setup).unwrap();
        assert_eq!(report.collisions.len(), 1, "{report:?}");
        let rub = &report.collisions[0];
        assert_eq!(rub.what, Hit::Shank);
        assert!((rub.depth - 2.0).abs() < 1e-9, "{rub:?}");
        // Sticking out only 4 mm: the 20 mm holder hits the top.
        let report = simulate(&side, &mill(3.0, 4.0), &setup).unwrap();
        let holder = report
            .collisions
            .iter()
            .find(|c| c.what == Hit::Holder)
            .unwrap();
        assert!((holder.depth - 1.0).abs() < 1e-9, "{holder:?}");
        // A plunge deeper than the flutes is fine: the shank follows the
        // hole the flutes made.
        let plunge = program(vec![rapid(10.0, 10.0, 2.0), cut(10.0, 10.0, -5.0)]);
        let report = simulate(&plunge, &mill(3.0, 30.0), &setup).unwrap();
        assert!(report.collisions.is_empty(), "{report:?}");
    }
}
