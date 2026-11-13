// SPDX-License-Identifier: AGPL-3.0-or-later
//! Layer 3 milling process: plans milling operations and produces neutral
//! toolpaths (cam-core). The post never sees the model.
//!
//! First slice (decision 0013): the holes. From every hole feature of the
//! rebuilt part it plans, in this order, a spot drill for every hole, the
//! drills (pecking deep holes), rigid tapping for threaded holes, and
//! reaming for holes with a fit, using a tool library. Holes it cannot make
//! yet (counterbores, holes not facing the spindle) are reported, never
//! silently skipped.
//!
//! UNVERIFIED: these operations have not cut a real part (docs/safety.md).

use std::collections::BTreeMap;
use std::fmt;

use cndt_cam_core::{
    ArcDirection, Coolant, Cycle, CycleKind, Event, Move, Operation, Program, SpindleDirection,
    Step,
};
use cndt_geom::Point3;
use cndt_model::cam::{OperationDef, SetupDef, Side};
use cndt_model::{Document, Feature, FrameStatus, HoleDepth, Rebuilt, threads};

pub use cndt_model::cam::{Tool, ToolKind};

/// The tools a program may use, by number.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolLibrary {
    /// Every tool, by number (T and H).
    pub tools: BTreeMap<u32, Tool>,
}

impl ToolLibrary {
    /// The default library of decision 0013.
    #[must_use]
    pub fn default_aluminium() -> Self {
        Self {
            tools: cndt_model::cam::default_tools(),
        }
    }

    fn find(&self, wanted: impl Fn(&Tool) -> bool) -> Option<(u32, &Tool)> {
        self.tools
            .iter()
            .find(|(_, tool)| wanted(tool))
            .map(|(n, t)| (*n, t))
    }

    fn drill(&self, diameter: f64) -> Option<(u32, &Tool)> {
        self.find(|t| t.kind == ToolKind::Drill && (t.diameter - diameter).abs() < 1e-6)
    }
}

/// Where the part sits on the machine for one setup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setup {
    /// The work-offset origin in model coordinates. For the top side,
    /// program X, Y and Z are model coordinates minus this; for the bottom
    /// side (flipped about X), program Y and Z are mirrored.
    pub origin: Point3,
    /// Which side faces the spindle.
    pub side: Side,
    /// Work offset, 1 (G54) to 6 (G59).
    pub work_offset: u8,
    /// Height above the origin for moves between holes.
    pub safe_z: f64,
    /// Height above each hole where feeding starts (the R plane).
    pub clearance: f64,
}

impl Setup {
    /// The first setup of a part: G54 at its front-left corner, Z0 on its
    /// top face (decision 0013).
    #[must_use]
    pub fn top_of(rebuilt: &Rebuilt) -> Option<Self> {
        let bounds = rebuilt.properties?.bounds;
        Some(Self {
            origin: Point3::new(bounds.min.x, bounds.min.y, bounds.max.z),
            side: Side::Top,
            work_offset: 1,
            safe_z: 10.0,
            clearance: 2.0,
        })
    }

    /// A stored setup on the rebuilt part: G-offset at the front-left corner
    /// of the side machined, Z0 on that face (decision 0014).
    #[must_use]
    pub fn from_def(def: &SetupDef, rebuilt: &Rebuilt) -> Option<Self> {
        let bounds = rebuilt.properties?.bounds;
        let origin = match def.side {
            Side::Top => Point3::new(bounds.min.x, bounds.min.y, bounds.max.z),
            // Flipped about X: the old back edge is now at the front.
            Side::Bottom => Point3::new(bounds.min.x, bounds.max.y, bounds.min.z),
        };
        Some(Self {
            origin,
            side: def.side,
            work_offset: def.work_offset,
            safe_z: 10.0,
            clearance: 2.0,
        })
    }

    /// A program point in model coordinates (the inverse of
    /// [`Setup::program_point`]).
    #[must_use]
    pub fn model_point(&self, p: Point3) -> Point3 {
        let o = self.origin;
        match self.side {
            Side::Top => Point3::new(p.x + o.x, p.y + o.y, p.z + o.z),
            Side::Bottom => Point3::new(p.x + o.x, o.y - p.y, o.z - p.z),
        }
    }

    /// A model point in program coordinates.
    pub fn program_point(&self, p: Point3) -> Point3 {
        let o = self.origin;
        match self.side {
            Side::Top => Point3::new(round3(p.x - o.x), round3(p.y - o.y), round3(p.z - o.z)),
            Side::Bottom => Point3::new(round3(p.x - o.x), round3(o.y - p.y), round3(o.z - p.z)),
        }
    }

    /// The model direction the spindle looks along, as the normal a hole's
    /// sketch must have to face it.
    const fn facing(&self) -> f64 {
        match self.side {
            Side::Top => 1.0,
            Side::Bottom => -1.0,
        }
    }
}

/// Why a program could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CamError {
    /// There is no part to machine.
    NoPart,
    /// No tool in the library fits; says which is needed.
    NoTool(String),
    /// Nothing to do.
    Nothing,
    /// No such setup.
    UnknownSetup(u32),
    /// The document has no setups yet.
    NoSetups,
    /// Facing needs the stock to be set.
    NoStock,
    /// A contour cannot be made; says why.
    Contour(String),
}

impl fmt::Display for CamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPart => write!(f, "there is no part to machine yet"),
            Self::NoTool(what) => write!(f, "the tool library has no {what}; add one"),
            Self::Nothing => write!(f, "the part has no holes this setup can make"),
            Self::UnknownSetup(id) => write!(f, "there is no setup {id}"),
            Self::Contour(reason) => write!(f, "the contour cannot be made: {reason}"),
            Self::NoStock => write!(f, "facing needs the stock; set it first (cam.set_stock)"),
            Self::NoSetups => write!(
                f,
                "the document has no setups yet; add one (cam.add_setup) and give it operations"
            ),
        }
    }
}

impl std::error::Error for CamError {}

/// A planned program and what it could not do.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// The program.
    pub program: Program,
    /// Where the setup puts the part, for previews.
    pub setup: Option<Setup>,
    /// Things left out, with reasons, for the user to see.
    pub notes: Vec<String>,
}

/// One hole to make, in program coordinates.
#[derive(Debug, Clone)]
struct Hole {
    at: Point3,
    /// Full-diameter depth below the hole's top; `None` for through.
    depth: Option<f64>,
    /// The finished diameter.
    diameter: f64,
    thread: Option<(&'static threads::ThreadSize, f64)>,
    reamed: bool,
    /// Counterbore diameter and depth.
    counterbore: Option<(f64, f64)>,
}

/// How far a drill's 118° point reaches below its full diameter.
fn point_length(diameter: f64) -> f64 {
    diameter / 2.0 / 59f64.to_radians().tan()
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Plans the hole operations for one setup.
///
/// # Errors
///
/// [`CamError`] when there is no part, no holes, or a needed tool is
/// missing from the library.
pub fn hole_program(
    document: &Document,
    rebuilt: &Rebuilt,
    setup: &Setup,
    library: &ToolLibrary,
    name: &str,
) -> Result<Planned, CamError> {
    let bounds = rebuilt.properties.ok_or(CamError::NoPart)?.bounds;
    // The far side of the part, in program Z.
    let bottom = round3(-(bounds.max.z - bounds.min.z));
    let mut notes = Vec::new();
    let mut holes: Vec<Hole> = Vec::new();
    for entry in document.features() {
        let Feature::Hole {
            name,
            sketch,
            points,
            spec,
        } = &entry.feature
        else {
            continue;
        };
        if !matches!(
            rebuilt.status.get(&entry.id),
            Some(cndt_model::FeatureStatus::Ok)
        ) {
            notes.push(format!("{name}: skipped, it does not build"));
            continue;
        }
        let Some(FrameStatus::Ok { frame }) = rebuilt.frames.get(sketch) else {
            continue;
        };
        if (frame.normal[2] - setup.facing()).abs() > 1e-9 {
            notes.push(format!(
                "{name}: skipped, its holes do not face the spindle in this setup"
            ));
            continue;
        }
        if spec.tip_angle >= 180.0 - 1e-9 && matches!(spec.depth, HoleDepth::Blind(_)) {
            notes.push(format!(
                "{name}: the model has a flat bottom, but the drill's point goes deeper than the full-diameter depth; a flat bottom needs milling"
            ));
        }
        let Some(record) = document.sketch(*sketch) else {
            continue;
        };
        for point in points {
            let Some(p) = record.sketch.point(*point) else {
                continue;
            };
            let at = setup.program_point(frame.at(p.x, p.y));
            let thread = spec
                .thread
                .as_ref()
                .and_then(|t| threads::thread(&t.size).map(|size| (size, t.depth)));
            let depth = match spec.depth {
                HoleDepth::Blind(d) => Some(d),
                HoleDepth::Through => None,
            };
            holes.push(Hole {
                at,
                depth,
                diameter: spec.diameter,
                thread,
                reamed: spec.fit.is_some(),
                counterbore: spec.counterbore.map(|c| (c.diameter, c.depth)),
            });
        }
    }
    if holes.is_empty() {
        return Err(CamError::Nothing);
    }
    let mut operations = Vec::new();

    // Spot every hole: to a depth that leaves a small chamfer on the drill.
    let spot = library
        .find(|t| t.kind == ToolKind::SpotDrill)
        .ok_or_else(|| CamError::NoTool(String::from("spot drill")))?;
    let mut by_depth: BTreeMap<u64, Vec<Point3>> = BTreeMap::new();
    for hole in &holes {
        let drill = if hole.reamed {
            hole.diameter - 0.2
        } else {
            hole.diameter
        };
        let depth = round3(drill / 2.0 + 0.2);
        by_depth
            .entry((depth * 1000.0) as u64)
            .or_default()
            .push(hole.at);
    }
    let cycles = by_depth
        .into_iter()
        .map(|(depth, at)| (CycleKind::Drill, at, -(depth as f64) / 1000.0, spot.1.feed))
        .collect();
    operations.push(operation("Spot drill", spot, setup, cycles));

    // Drill each size, deepest-first within a size never matters: one cycle
    // per depth.
    let mut drills: BTreeMap<u64, Vec<&Hole>> = BTreeMap::new();
    for hole in &holes {
        let size = if hole.reamed {
            hole.diameter - 0.2
        } else {
            hole.diameter
        };
        drills
            .entry((size * 1000.0).round() as u64)
            .or_default()
            .push(hole);
    }
    for (size, group) in drills {
        let diameter = size as f64 / 1000.0;
        let tool = library
            .drill(diameter)
            .ok_or_else(|| CamError::NoTool(format!("{diameter} mm drill")))?;
        let mut by_depth: BTreeMap<i64, Vec<Point3>> = BTreeMap::new();
        for hole in group {
            // Through: 1 mm past the bottom, plus the point.
            let z = match hole.depth {
                Some(d) => hole.at.z - d - point_length(diameter),
                None => bottom - 1.0 - point_length(diameter),
            };
            by_depth
                .entry((round3(z) * 1000.0) as i64)
                .or_default()
                .push(hole.at);
        }
        let cycles = by_depth
            .into_iter()
            .map(|(z, at)| {
                let z = z as f64 / 1000.0;
                let depth = at.first().map_or(0.0, |p| p.z) - z;
                // Peck holes deeper than three diameters, one diameter a peck.
                let kind = if depth > 3.0 * diameter {
                    CycleKind::Peck { step: diameter }
                } else {
                    CycleKind::Drill
                };
                (kind, at, z, tool.1.feed)
            })
            .collect();
        operations.push(operation(
            &format!("Drill {diameter} mm"),
            tool,
            setup,
            cycles,
        ));
    }

    // Counterbores: milled with a helix, after the through drill.
    let mut counterbores: BTreeMap<(u64, u64), Vec<&Hole>> = BTreeMap::new();
    for hole in &holes {
        if let Some((d, depth)) = hole.counterbore {
            counterbores
                .entry(((d * 1000.0).round() as u64, (depth * 1000.0).round() as u64))
                .or_default()
                .push(hole);
        }
    }
    for ((d, depth), group) in counterbores {
        let (diameter, depth) = (d as f64 / 1000.0, depth as f64 / 1000.0);
        // One circle must also clear the middle: at least half the bore.
        let (number, tool) = library
            .tools
            .iter()
            .filter(|(_, t)| {
                t.kind == ToolKind::EndMill
                    && t.diameter >= diameter / 2.0
                    && t.diameter < diameter - 0.2
            })
            .max_by(|a, b| a.1.diameter.total_cmp(&b.1.diameter).then(b.0.cmp(a.0)))
            .map(|(n, t)| (*n, t))
            .ok_or_else(|| {
                CamError::NoTool(format!(
                    "end mill between {} and {} mm for the {diameter} mm counterbores",
                    diameter / 2.0,
                    diameter - 0.2
                ))
            })?;
        operations.push(counterbore_op(number, tool, setup, &group, diameter, depth));
    }

    // Tap: rigid, feed = rpm × pitch.
    let mut taps: BTreeMap<&str, Vec<&Hole>> = BTreeMap::new();
    for hole in &holes {
        if let Some((size, _)) = hole.thread {
            taps.entry(size.designation).or_default().push(hole);
        }
    }
    for (designation, group) in taps {
        let tool = library
            .find(|t| {
                t.kind
                    == ToolKind::Tap {
                        thread: designation.to_owned(),
                    }
            })
            .ok_or_else(|| CamError::NoTool(format!("{designation} tap")))?;
        let pitch = group
            .first()
            .and_then(|h| h.thread)
            .map_or(1.0, |(s, _)| s.pitch);
        let mut by_depth: BTreeMap<i64, Vec<Point3>> = BTreeMap::new();
        for hole in group {
            let depth = hole.thread.map_or(0.0, |(_, d)| d);
            by_depth
                .entry((round3(hole.at.z - depth) * 1000.0) as i64)
                .or_default()
                .push(hole.at);
        }
        let mut tapping = tool.1.clone();
        tapping.feed = tapping.rpm * pitch;
        let cycles = by_depth
            .into_iter()
            .map(|(z, at)| {
                (
                    CycleKind::Tap { pitch },
                    at,
                    z as f64 / 1000.0,
                    tapping.feed,
                )
            })
            .collect();
        operations.push(operation(
            &format!("Tap {designation}"),
            (tool.0, &tapping),
            setup,
            cycles,
        ));
    }

    // Ream holes with a fit.
    let mut reams: BTreeMap<u64, Vec<&Hole>> = BTreeMap::new();
    for hole in holes.iter().filter(|h| h.reamed) {
        reams
            .entry((hole.diameter * 1000.0).round() as u64)
            .or_default()
            .push(hole);
    }
    for (size, group) in reams {
        let diameter = size as f64 / 1000.0;
        let tool = library
            .find(|t| t.kind == ToolKind::Reamer && (t.diameter - diameter).abs() < 1e-6)
            .ok_or_else(|| CamError::NoTool(format!("{diameter} mm reamer")))?;
        let mut by_depth: BTreeMap<i64, Vec<Point3>> = BTreeMap::new();
        for hole in group {
            let z = match hole.depth {
                Some(d) => hole.at.z - d,
                None => bottom - 1.0,
            };
            by_depth
                .entry((round3(z) * 1000.0) as i64)
                .or_default()
                .push(hole.at);
        }
        let cycles = by_depth
            .into_iter()
            .map(|(z, at)| (CycleKind::Ream, at, z as f64 / 1000.0, tool.1.feed))
            .collect();
        operations.push(operation(
            &format!("Ream {diameter} mm"),
            tool,
            setup,
            cycles,
        ));
    }

    let program = Program {
        name: name.to_owned(),
        number: 1000,
        work_offset: setup.work_offset,
        operations,
    };
    Ok(Planned {
        program,
        notes,
        setup: None,
    })
}

/// Most material a face mill takes in one pass, in millimetres.
const FACE_DEPTH_PER_PASS: f64 = 1.0;
/// Sideways step between passes, as a fraction of the cutter diameter.
const FACE_STEPOVER: f64 = 0.7;
/// How far past the stock every plunge and step-over happens.
const FACE_CLEAR: f64 = 2.0;

/// Faces the stock down to Z0 with zig-zag passes along X: each level is
/// plunged outside the stock, every row runs fully past it, and rows are
/// joined outside it, so the cutter never plunges into material.
fn face(
    document: &Document,
    rebuilt: &Rebuilt,
    setup: &Setup,
    library: &ToolLibrary,
    tool: Option<u32>,
) -> Result<Operation, CamError> {
    let stock = document.cam().stock.ok_or(CamError::NoStock)?;
    let bounds = rebuilt.properties.ok_or(CamError::NoPart)?.bounds;
    let (number, cutter) = match tool {
        Some(n) => library
            .tools
            .get(&n)
            .map(|t| (n, t))
            .ok_or_else(|| CamError::NoTool(format!("tool {n}")))?,
        None => library
            .find(|t| t.kind == ToolKind::FaceMill)
            .ok_or_else(|| CamError::NoTool(String::from("face mill")))?,
    };
    let allowance = match setup.side {
        Side::Top => stock.top,
        Side::Bottom => stock.bottom,
    };
    let [length, width, _] = bounds.size();
    // The stock outline in program coordinates (the same for both sides:
    // flipping about X maps the box onto itself).
    let (x_min, x_max) = (-stock.side, length + stock.side);
    let (y_min, y_max) = (-stock.side, width + stock.side);
    let r = cutter.diameter / 2.0;
    let step = cutter.diameter * FACE_STEPOVER;
    // Rows: the first overhangs the near edge by a tenth of the cutter, and
    // they continue until the far edge is covered.
    let mut rows = Vec::new();
    let mut y = y_min + r - 0.1 * cutter.diameter;
    loop {
        rows.push(round3(y));
        if y + r >= y_max + 0.1 * cutter.diameter || rows.len() > 1000 {
            break;
        }
        y += step;
    }
    let (left, right) = (
        round3(x_min - r - FACE_CLEAR),
        round3(x_max + r + FACE_CLEAR),
    );
    // Levels from the stock top down to Z0, equal and at most 1 mm each.
    let passes = ((allowance / FACE_DEPTH_PER_PASS).ceil() as usize).max(1);
    let levels: Vec<f64> = (1..=passes)
        .map(|k| round3(allowance - allowance * k as f64 / passes as f64))
        .collect();
    let top = round3(allowance);
    let safe = round3(setup.safe_z.max(top + setup.clearance));
    let mut steps = vec![
        Step::Event(Event::ToolChange {
            number,
            description: if cutter.description.is_empty() {
                format!("tool {number}")
            } else {
                cutter.description.clone()
            },
        }),
        Step::Event(Event::SpindleOn {
            rpm: cutter.rpm,
            direction: SpindleDirection::Clockwise,
        }),
        Step::Event(Event::Coolant {
            mode: Coolant::Flood,
        }),
    ];
    let plunge = cutter.feed / 3.0;
    for z in levels {
        let first = rows.first().copied().unwrap_or(0.0);
        steps.push(Step::Move(Move::Rapid {
            to: Point3::new(left, first, safe),
        }));
        steps.push(Step::Move(Move::Rapid {
            to: Point3::new(left, first, top + setup.clearance),
        }));
        steps.push(Step::Move(Move::Linear {
            to: Point3::new(left, first, z),
            feed: plunge,
        }));
        for (i, &row) in rows.iter().enumerate() {
            let (from, to) = if i % 2 == 0 {
                (left, right)
            } else {
                (right, left)
            };
            if i > 0 {
                // Step over outside the stock.
                steps.push(Step::Move(Move::Linear {
                    to: Point3::new(from, row, z),
                    feed: cutter.feed,
                }));
            }
            steps.push(Step::Move(Move::Linear {
                to: Point3::new(to, row, z),
                feed: cutter.feed,
            }));
        }
        let end_x = if rows.len() % 2 == 1 { right } else { left };
        let last = rows.last().copied().unwrap_or(0.0);
        steps.push(Step::Move(Move::Rapid {
            to: Point3::new(end_x, last, safe),
        }));
    }
    steps.push(Step::Event(Event::Coolant { mode: Coolant::Off }));
    Ok(Operation {
        name: format!("Face {} mm", round3(allowance)),
        steps,
    })
}

/// Depth per contour pass, as a multiple of the cutter diameter.
const CONTOUR_DEPTH_PER_PASS: f64 = 1.0;

/// One piece of a cutter path in program coordinates.
#[derive(Debug, Clone, Copy)]
enum Piece {
    Line {
        start: Point3,
        end: Point3,
    },
    Arc {
        start: Point3,
        end: Point3,
        center: Point3,
        ccw: bool,
    },
}

impl Piece {
    fn reversed(self) -> Self {
        match self {
            Self::Line { start, end } => Self::Line {
                start: end,
                end: start,
            },
            Self::Arc {
                start,
                end,
                center,
                ccw,
            } => Self::Arc {
                start: end,
                end: start,
                center,
                ccw: !ccw,
            },
        }
    }

    /// Points along it for area and length sums.
    fn samples(&self) -> Vec<Point3> {
        match *self {
            Self::Line { start, .. } => vec![start],
            Self::Arc {
                start,
                end,
                center,
                ccw,
            } => {
                let r = (start.x - center.x).hypot(start.y - center.y);
                let a0 = (start.y - center.y).atan2(start.x - center.x);
                let a1 = (end.y - center.y).atan2(end.x - center.x);
                let tau = std::f64::consts::TAU;
                let sweep = if ccw {
                    (a1 - a0).rem_euclid(tau)
                } else {
                    -(a0 - a1).rem_euclid(tau)
                };
                let sweep = if sweep.abs() < 1e-12 {
                    if ccw { tau } else { -tau }
                } else {
                    sweep
                };
                (0..16)
                    .map(|k| {
                        let a = a0 + sweep * f64::from(k) / 16.0;
                        Point3::new(center.x + r * a.cos(), center.y + r * a.sin(), start.z)
                    })
                    .collect()
            }
        }
    }
}

/// The one face named `token`.
fn unique_face(rebuilt: &Rebuilt, token: &str) -> Result<u32, CamError> {
    let found: Vec<usize> = rebuilt
        .faces
        .iter()
        .enumerate()
        .filter(|(_, names)| names.iter().any(|n| n.to_string() == token))
        .map(|(i, _)| i)
        .collect();
    match found.as_slice() {
        [one] => u32::try_from(*one).map_err(|_| CamError::Contour(String::from("too many faces"))),
        [] => Err(CamError::Contour(format!(
            "the face {token} no longer exists; pick it again"
        ))),
        _ => Err(CamError::Contour(format!(
            "the face {token} is split into {} faces; pick one",
            found.len()
        ))),
    }
}

/// The outside profile: the outline of a flat face, offset by the cutter's
/// radius, climb-milled (clockwise) in equal levels no deeper than the
/// cutter diameter. Each level starts and ends at the middle of the longest
/// straight side, fed in square to the wall from a point outside the
/// stock, where the cutter also plunges.
#[allow(clippy::too_many_arguments)]
fn contour(
    document: &Document,
    rebuilt: &Rebuilt,
    kernel: &dyn cndt_kernel::Kernel,
    setup: &Setup,
    library: &ToolLibrary,
    face: &str,
    tool: Option<u32>,
    depth: Option<f64>,
) -> Result<Operation, CamError> {
    let body = rebuilt.body.as_ref().ok_or(CamError::NoPart)?;
    let (number, cutter) = match tool {
        Some(n) => library
            .tools
            .get(&n)
            .map(|t| (n, t))
            .ok_or_else(|| CamError::NoTool(format!("tool {n}")))?,
        None => library
            .find(|t| t.kind == ToolKind::EndMill)
            .ok_or_else(|| CamError::NoTool(String::from("end mill")))?,
    };
    let r = cutter.diameter / 2.0;
    let index = unique_face(rebuilt, face)?;
    let (point, normal) = kernel
        .face_plane(body, index)
        .map_err(|e| CamError::Contour(e.to_string()))?;
    if normal[2].abs() < 1.0 - 1e-9 {
        return Err(CamError::Contour(String::from(
            "the face is not square to the spindle",
        )));
    }
    let outline = kernel
        .face_outline(body, index, r)
        .map_err(|e| CamError::Contour(e.to_string()))?;
    let flip = setup.side == Side::Bottom;
    let mut path: Vec<Piece> = outline
        .into_iter()
        .map(|s| match s {
            cndt_kernel::OutlineSegment::Line { start, end } => Piece::Line {
                start: setup.program_point(start),
                end: setup.program_point(end),
            },
            // Turning the part over mirrors the arcs' sense.
            cndt_kernel::OutlineSegment::Arc {
                start,
                end,
                center,
                ccw,
            } => Piece::Arc {
                start: setup.program_point(start),
                end: setup.program_point(end),
                center: setup.program_point(center),
                ccw: ccw != flip,
            },
        })
        .collect();
    // Climb milling outside a part with a clockwise spindle: go clockwise.
    let samples: Vec<Point3> = path.iter().flat_map(Piece::samples).collect();
    let n = samples.len();
    let area: f64 = (0..n)
        .filter_map(|i| Some((samples.get(i)?, samples.get((i + 1) % n)?)))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>()
        / 2.0;
    if area > 0.0 {
        path = path.into_iter().rev().map(Piece::reversed).collect();
    }
    // Start in the middle of the longest straight side.
    let longest = path
        .iter()
        .enumerate()
        .filter_map(|(i, p)| match *p {
            Piece::Line { start, end } => Some((i, (end.x - start.x).hypot(end.y - start.y))),
            Piece::Arc { .. } => None,
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
        .ok_or_else(|| {
            CamError::Contour(String::from(
                "the outline has no straight side to start from",
            ))
        })?;
    path.rotate_left(longest);
    let Some(Piece::Line { start, end }) = path.first().copied() else {
        return Err(CamError::Contour(String::from(
            "the outline has no straight side to start from",
        )));
    };
    let mid = Point3::new(
        round3((start.x + end.x) / 2.0),
        round3((start.y + end.y) / 2.0),
        0.0,
    );
    path.remove(0);
    path.insert(0, Piece::Line { start: mid, end });
    path.push(Piece::Line { start, end: mid });
    // Outward from a clockwise loop is to the left of travel.
    let length = (end.x - start.x).hypot(end.y - start.y);
    let (ux, uy) = ((end.x - start.x) / length, (end.y - start.y) / length);
    let lead = document.cam().stock.map_or(0.0, |s| s.side) + 2.0;
    let entry = Point3::new(round3(mid.x - uy * lead), round3(mid.y + ux * lead), 0.0);

    let face_z = setup.program_point(point).z;
    let total = depth.unwrap_or(-face_z);
    if !(total.is_finite() && total > 0.0) {
        return Err(CamError::Contour(format!(
            "the depth must be more than zero, not {total}"
        )));
    }
    let passes = ((total / (cutter.diameter * CONTOUR_DEPTH_PER_PASS)).ceil() as usize).max(1);
    let levels: Vec<f64> = (1..=passes)
        .map(|k| round3(-total * k as f64 / passes as f64))
        .collect();
    let stock_top = document.cam().stock.map_or(0.0, |s| match setup.side {
        Side::Top => s.top,
        Side::Bottom => s.bottom,
    });
    let safe = round3(setup.safe_z.max(stock_top + setup.clearance));
    let at = |p: Point3, z: f64| Point3::new(round3(p.x), round3(p.y), z);
    let mut steps = vec![
        Step::Event(Event::ToolChange {
            number,
            description: if cutter.description.is_empty() {
                format!("tool {number}")
            } else {
                cutter.description.clone()
            },
        }),
        Step::Event(Event::SpindleOn {
            rpm: cutter.rpm,
            direction: SpindleDirection::Clockwise,
        }),
        Step::Event(Event::Coolant {
            mode: Coolant::Flood,
        }),
        Step::Move(Move::Rapid {
            to: at(entry, safe),
        }),
        Step::Move(Move::Rapid {
            to: at(entry, round3(stock_top + setup.clearance)),
        }),
    ];
    let plunge = cutter.feed / 3.0;
    for z in levels {
        steps.push(Step::Move(Move::Linear {
            to: at(entry, z),
            feed: plunge,
        }));
        steps.push(Step::Move(Move::Linear {
            to: at(mid, z),
            feed: cutter.feed,
        }));
        for piece in &path {
            steps.push(Step::Move(match *piece {
                Piece::Line { end, .. } => Move::Linear {
                    to: at(end, z),
                    feed: cutter.feed,
                },
                Piece::Arc {
                    end, center, ccw, ..
                } => Move::Arc {
                    to: at(end, z),
                    center: at(center, z),
                    direction: if ccw {
                        ArcDirection::CounterClockwise
                    } else {
                        ArcDirection::Clockwise
                    },
                    feed: cutter.feed,
                },
            }));
        }
        steps.push(Step::Move(Move::Linear {
            to: at(entry, z),
            feed: cutter.feed,
        }));
    }
    steps.push(Step::Move(Move::Rapid {
        to: at(entry, safe),
    }));
    steps.push(Step::Event(Event::Coolant { mode: Coolant::Off }));
    Ok(Operation {
        name: format!("Contour {} mm deep", round3(total)),
        steps,
    })
}

/// Sideways step between pocket loops, as a fraction of the cutter.
const POCKET_STEPOVER: f64 = 0.4;
/// Depth per pocket level, as a fraction of the cutter.
const POCKET_DEPTH_PER_PASS: f64 = 0.5;
/// Ramp angle into a pocket.
const RAMP_DEGREES: f64 = 3.0;

/// A flat face's outline offset inwards by `distance`, in program
/// coordinates, anticlockwise (climb milling inside a pocket), starting at
/// the middle of its longest straight side if it has one.
fn inner_loop(
    kernel: &dyn cndt_kernel::Kernel,
    body: &cndt_kernel::Body,
    setup: &Setup,
    face: u32,
    distance: f64,
) -> Option<Vec<Piece>> {
    let outline = kernel.face_outline(body, face, -distance).ok()?;
    let flip = setup.side == Side::Bottom;
    let mut path: Vec<Piece> = outline
        .into_iter()
        .map(|s| match s {
            cndt_kernel::OutlineSegment::Line { start, end } => Piece::Line {
                start: setup.program_point(start),
                end: setup.program_point(end),
            },
            cndt_kernel::OutlineSegment::Arc {
                start,
                end,
                center,
                ccw,
            } => Piece::Arc {
                start: setup.program_point(start),
                end: setup.program_point(end),
                center: setup.program_point(center),
                ccw: ccw != flip,
            },
        })
        .collect();
    let samples: Vec<Point3> = path.iter().flat_map(Piece::samples).collect();
    let n = samples.len();
    let area: f64 = (0..n)
        .filter_map(|i| Some((samples.get(i)?, samples.get((i + 1) % n)?)))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum::<f64>();
    if area < 0.0 {
        path = path.into_iter().rev().map(Piece::reversed).collect();
    }
    let longest = path
        .iter()
        .enumerate()
        .filter_map(|(i, p)| match *p {
            Piece::Line { start, end } => Some((i, (end.x - start.x).hypot(end.y - start.y))),
            Piece::Arc { .. } => None,
        })
        .max_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, length)) = longest
        && length > 1e-6
    {
        path.rotate_left(i);
        if let Some(Piece::Line { start, end }) = path.first().copied() {
            let mid = Point3::new(
                round3((start.x + end.x) / 2.0),
                round3((start.y + end.y) / 2.0),
                start.z,
            );
            path.remove(0);
            path.insert(0, Piece::Line { start: mid, end });
            path.push(Piece::Line { start, end: mid });
        }
    }
    Some(path)
}

fn piece_length(piece: &Piece) -> f64 {
    let samples = match piece {
        Piece::Line { start, end } => vec![*start, *end],
        Piece::Arc { end, .. } => {
            let mut s = piece.samples();
            s.push(*end);
            s
        }
    };
    samples
        .windows(2)
        .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
        .sum()
}

/// The move that cuts along a piece, ending at height `z`.
fn piece_move(piece: &Piece, z: f64, feed: f64) -> Move {
    let at = |p: Point3| Point3::new(round3(p.x), round3(p.y), round3(z));
    match *piece {
        Piece::Line { end, .. } => Move::Linear { to: at(end), feed },
        Piece::Arc {
            end, center, ccw, ..
        } => Move::Arc {
            to: at(end),
            center: at(center),
            direction: if ccw {
                ArcDirection::CounterClockwise
            } else {
                ArcDirection::Clockwise
            },
            feed,
        },
    }
}

/// Clears a pocket: offset loops from the middle outwards, the last one a
/// cutter radius from the wall, in levels of half the cutter. Each level is
/// entered by a 3° ramp along the innermost loop, never by plunging.
fn pocket(
    rebuilt: &Rebuilt,
    kernel: &dyn cndt_kernel::Kernel,
    setup: &Setup,
    library: &ToolLibrary,
    floor: &str,
    tool: Option<u32>,
) -> Result<Operation, CamError> {
    let bad = |reason: String| CamError::Contour(reason);
    let body = rebuilt.body.as_ref().ok_or(CamError::NoPart)?;
    let index = unique_face(rebuilt, floor)?;
    let (point, normal) = kernel
        .face_plane(body, index)
        .map_err(|e| bad(e.to_string()))?;
    if (normal[2] - setup.facing()).abs() > 1e-9 {
        return Err(bad(String::from(
            "the pocket's floor does not face the spindle in this setup",
        )));
    }
    let depth = -setup.program_point(point).z;
    if depth.is_nan() || depth <= 1e-6 {
        return Err(bad(String::from("the pocket's floor is not below Z0")));
    }
    // The tightest corner the cutter must fit into.
    let outline = kernel
        .face_outline(body, index, 0.0)
        .map_err(|e| bad(e.to_string()))?;
    let tightest = outline
        .iter()
        .filter_map(|s| match s {
            cndt_kernel::OutlineSegment::Arc { start, center, .. } => {
                Some((start.x - center.x).hypot(start.y - center.y))
            }
            cndt_kernel::OutlineSegment::Line { .. } => None,
        })
        .fold(f64::INFINITY, f64::min);
    let fits = |t: &Tool| {
        t.diameter / 2.0 <= tightest + 1e-9
            && inner_loop(kernel, body, setup, index, t.diameter / 2.0).is_some()
    };
    let (number, cutter) = match tool {
        Some(n) => {
            let t = library
                .tools
                .get(&n)
                .ok_or_else(|| CamError::NoTool(format!("tool {n}")))?;
            if !fits(t) {
                return Err(bad(format!(
                    "tool {n} ({} mm) does not fit the pocket",
                    t.diameter
                )));
            }
            (n, t)
        }
        None => library
            .tools
            .iter()
            .filter(|(_, t)| t.kind == ToolKind::EndMill && fits(t))
            .max_by(|a, b| a.1.diameter.total_cmp(&b.1.diameter).then(b.0.cmp(a.0)))
            .map(|(n, t)| (*n, t))
            .ok_or_else(|| {
                CamError::NoTool(String::from("end mill small enough for the pocket"))
            })?,
    };
    let r = cutter.diameter / 2.0;
    // Loops from the wall inwards, then cut from the middle outwards.
    let mut loops = Vec::new();
    let mut offset = r;
    while let Some(path) = inner_loop(kernel, body, setup, index, offset) {
        loops.push(path);
        offset += cutter.diameter * POCKET_STEPOVER;
        if loops.len() > 500 {
            break;
        }
    }
    loops.reverse();
    let passes = ((depth / (cutter.diameter * POCKET_DEPTH_PER_PASS)).ceil() as usize).max(1);
    let levels: Vec<f64> = (1..=passes)
        .map(|k| round3(-depth * k as f64 / passes as f64))
        .collect();
    let safe = round3(setup.safe_z);
    let start = |path: &[Piece]| {
        path.first()
            .map_or(Point3::new(0.0, 0.0, 0.0), |p| match *p {
                Piece::Line { start, .. } | Piece::Arc { start, .. } => start,
            })
    };
    let innermost = loops.first().cloned().unwrap_or_default();
    let entry = start(&innermost);
    let lap: f64 = innermost.iter().map(piece_length).sum();
    let mut steps = vec![
        Step::Event(Event::ToolChange {
            number,
            description: if cutter.description.is_empty() {
                format!("tool {number}")
            } else {
                cutter.description.clone()
            },
        }),
        Step::Event(Event::SpindleOn {
            rpm: cutter.rpm,
            direction: SpindleDirection::Clockwise,
        }),
        Step::Event(Event::Coolant {
            mode: Coolant::Flood,
        }),
        Step::Move(Move::Rapid {
            to: Point3::new(entry.x, entry.y, safe),
        }),
    ];
    let ramp_feed = cutter.feed / 2.0;
    let mut above = 0.0;
    for z in levels {
        // Down to just above the last level (already cleared), then ramp.
        steps.push(Step::Move(Move::Rapid {
            to: Point3::new(entry.x, entry.y, round3(above + 0.5)),
        }));
        steps.push(Step::Move(Move::Linear {
            to: Point3::new(entry.x, entry.y, round3(above)),
            feed: ramp_feed,
        }));
        let drop = above - z;
        let per_lap = lap * RAMP_DEGREES.to_radians().tan();
        if per_lap > 1e-6 {
            let laps = (drop / per_lap).ceil().max(1.0) as usize;
            let total = lap * laps as f64;
            let mut travelled = 0.0;
            for _ in 0..laps {
                for piece in &innermost {
                    travelled += piece_length(piece);
                    let h = above - drop * (travelled / total).min(1.0);
                    steps.push(Step::Move(piece_move(piece, h, ramp_feed)));
                }
            }
        } else {
            // A loop too short to ramp along: feed straight down slowly.
            steps.push(Step::Move(Move::Linear {
                to: Point3::new(entry.x, entry.y, z),
                feed: ramp_feed / 2.0,
            }));
        }
        // One flat lap to clear the ramp, then each loop outwards.
        for (k, path) in loops.iter().enumerate() {
            if k > 0 {
                let s = start(path);
                steps.push(Step::Move(Move::Linear {
                    to: Point3::new(round3(s.x), round3(s.y), z),
                    feed: cutter.feed,
                }));
            }
            for piece in path {
                steps.push(Step::Move(piece_move(piece, z, cutter.feed)));
            }
        }
        // Back to the middle for the next level.
        steps.push(Step::Move(Move::Rapid {
            to: Point3::new(entry.x, entry.y, round3(z + 0.5)),
        }));
        above = z;
    }
    steps.push(Step::Move(Move::Rapid {
        to: Point3::new(entry.x, entry.y, safe),
    }));
    steps.push(Step::Event(Event::Coolant { mode: Coolant::Off }));
    Ok(Operation {
        name: format!("Pocket {} mm deep", round3(depth)),
        steps,
    })
}

/// How far below a chamfer the 90° tool's tip goes, so it never rubs.
const CHAMFER_TIP_CLEARANCE: f64 = 1.0;

/// Mills 45° chamfers of `width` round every edge of a flat face with a 90°
/// chamfer mill. The face, as modelled, already has the chamfers, so its
/// loops are inset by `width`; with the tip `t` below the face, the
/// cutter's centre runs on those loops offset by `t` into the air. Outer
/// loops run clockwise and holes anticlockwise (climb). Each loop is
/// entered from the air: outside the part, or the middle of the opening.
fn chamfer_mill(
    rebuilt: &Rebuilt,
    kernel: &dyn cndt_kernel::Kernel,
    setup: &Setup,
    library: &ToolLibrary,
    face: &str,
    width: f64,
    tool: Option<u32>,
) -> Result<Operation, CamError> {
    let bad = |reason: String| CamError::Contour(reason);
    let body = rebuilt.body.as_ref().ok_or(CamError::NoPart)?;
    let (number, cutter) = match tool {
        Some(n) => library
            .tools
            .get(&n)
            .map(|t| (n, t))
            .ok_or_else(|| CamError::NoTool(format!("tool {n}")))?,
        None => library
            .find(|t| t.kind == ToolKind::ChamferMill)
            .ok_or_else(|| CamError::NoTool(String::from("chamfer mill")))?,
    };
    if !(width.is_finite() && width > 0.0) {
        return Err(bad(format!(
            "the chamfer width must be more than zero, not {width}"
        )));
    }
    let t = width + CHAMFER_TIP_CLEARANCE;
    if t > cutter.diameter / 2.0 {
        return Err(bad(format!(
            "a {width} mm chamfer is too wide for the {} mm chamfer mill",
            cutter.diameter
        )));
    }
    let index = unique_face(rebuilt, face)?;
    let (point, normal) = kernel
        .face_plane(body, index)
        .map_err(|e| bad(e.to_string()))?;
    if (normal[2] - setup.facing()).abs() > 1e-9 {
        return Err(bad(String::from(
            "the chamfered face does not face the spindle in this setup",
        )));
    }
    let face_z = setup.program_point(point).z;
    let z = round3(face_z - t);
    let flip = setup.side == Side::Bottom;
    let loops = kernel
        .face_loops(body, index, t)
        .map_err(|e| bad(e.to_string()))?;
    let mut paths: Vec<(f64, Vec<Piece>)> = loops
        .into_iter()
        .map(|l| {
            let path: Vec<Piece> = l
                .into_iter()
                .map(|s| match s {
                    cndt_kernel::OutlineSegment::Line { start, end } => Piece::Line {
                        start: setup.program_point(start),
                        end: setup.program_point(end),
                    },
                    cndt_kernel::OutlineSegment::Arc {
                        start,
                        end,
                        center,
                        ccw,
                    } => Piece::Arc {
                        start: setup.program_point(start),
                        end: setup.program_point(end),
                        center: setup.program_point(center),
                        ccw: ccw != flip,
                    },
                })
                .collect();
            let samples: Vec<Point3> = path.iter().flat_map(Piece::samples).collect();
            let n = samples.len();
            let area: f64 = (0..n)
                .filter_map(|i| Some((samples.get(i)?, samples.get((i + 1) % n)?)))
                .map(|(a, b)| a.x * b.y - b.x * a.y)
                .sum::<f64>()
                / 2.0;
            (area, path)
        })
        .collect();
    // The outer loop encloses the most.
    let outer = paths
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.0.abs().total_cmp(&b.1.0.abs()))
        .map(|(i, _)| i)
        .ok_or_else(|| bad(String::from("the face has no boundary")))?;
    let safe = round3(setup.safe_z);
    let mut steps = vec![
        Step::Event(Event::ToolChange {
            number,
            description: if cutter.description.is_empty() {
                format!("tool {number}")
            } else {
                cutter.description.clone()
            },
        }),
        Step::Event(Event::SpindleOn {
            rpm: cutter.rpm,
            direction: SpindleDirection::Clockwise,
        }),
        Step::Event(Event::Coolant {
            mode: Coolant::Flood,
        }),
    ];
    // Outer loop first, then the openings in a fixed order.
    let order: Vec<usize> = std::iter::once(outer)
        .chain((0..paths.len()).filter(|i| *i != outer))
        .collect();
    for i in order {
        let Some((area, path)) = paths.get_mut(i) else {
            continue;
        };
        let is_outer = i == outer;
        // Clockwise outside, anticlockwise inside.
        if (is_outer && *area > 0.0) || (!is_outer && *area < 0.0) {
            *path = std::mem::take(path)
                .into_iter()
                .rev()
                .map(Piece::reversed)
                .collect();
        }
        // Start mid-way along the longest line, or at the first piece.
        if let Some((k, length)) = path
            .iter()
            .enumerate()
            .filter_map(|(k, p)| match *p {
                Piece::Line { start, end } => Some((k, (end.x - start.x).hypot(end.y - start.y))),
                Piece::Arc { .. } => None,
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            && length > 1e-6
        {
            path.rotate_left(k);
            if let Some(Piece::Line { start, end }) = path.first().copied() {
                let mid = Point3::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0, start.z);
                path.remove(0);
                path.insert(0, Piece::Line { start: mid, end });
                path.push(Piece::Line { start, end: mid });
            }
        }
        let Some(first) = path.first().copied() else {
            continue;
        };
        // Travel direction at the start, and the lead into the air to its left.
        let (start, lead) = match first {
            Piece::Line { start, end } => {
                let length = (end.x - start.x).hypot(end.y - start.y).max(1e-12);
                let (dx, dy) = ((end.x - start.x) / length, (end.y - start.y) / length);
                (
                    start,
                    Point3::new(start.x - dy * 2.0, start.y + dx * 2.0, 0.0),
                )
            }
            // A round opening: lead from its centre.
            Piece::Arc { start, center, .. } => (start, center),
        };
        let at = |p: Point3, h: f64| Point3::new(round3(p.x), round3(p.y), h);
        steps.push(Step::Move(Move::Rapid { to: at(lead, safe) }));
        steps.push(Step::Move(Move::Rapid {
            to: at(lead, round3(face_z + setup.clearance)),
        }));
        steps.push(Step::Move(Move::Linear {
            to: at(lead, z),
            feed: cutter.feed / 3.0,
        }));
        steps.push(Step::Move(Move::Linear {
            to: at(start, z),
            feed: cutter.feed,
        }));
        for piece in path.iter() {
            steps.push(Step::Move(piece_move(piece, z, cutter.feed)));
        }
        steps.push(Step::Move(Move::Linear {
            to: at(lead, z),
            feed: cutter.feed,
        }));
        steps.push(Step::Move(Move::Rapid { to: at(lead, safe) }));
    }
    steps.push(Step::Event(Event::Coolant { mode: Coolant::Off }));
    Ok(Operation {
        name: format!("Chamfer {width} mm"),
        steps,
    })
}

/// One straight piece of a toolpath preview, in model coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewSegment {
    /// A rapid move rather than a cut.
    pub rapid: bool,
    /// Where it starts.
    pub from: Point3,
    /// Where it ends.
    pub to: Point3,
}

/// A planned program as straight pieces in model coordinates, for drawing:
/// arcs are split into short chords, cycles into their rapids and feeds.
#[must_use]
pub fn preview(program: &Program, setup: &Setup) -> Vec<PreviewSegment> {
    let mut out = Vec::new();
    let mut at: Option<Point3> = None;
    let push = |out: &mut Vec<PreviewSegment>, from: Option<Point3>, to: Point3, rapid: bool| {
        if let Some(from) = from {
            out.push(PreviewSegment {
                rapid,
                from: setup.model_point(from),
                to: setup.model_point(to),
            });
        }
    };
    for step in program.operations.iter().flat_map(|op| &op.steps) {
        let Step::Move(motion) = step else { continue };
        match motion {
            Move::Rapid { to } => {
                push(&mut out, at, *to, true);
                at = Some(*to);
            }
            Move::Linear { to, .. } => {
                push(&mut out, at, *to, false);
                at = Some(*to);
            }
            Move::Arc {
                to,
                center,
                direction,
                ..
            } => {
                let Some(start) = at else {
                    at = Some(*to);
                    continue;
                };
                let r = (start.x - center.x).hypot(start.y - center.y);
                let a0 = (start.y - center.y).atan2(start.x - center.x);
                let a1 = (to.y - center.y).atan2(to.x - center.x);
                let tau = std::f64::consts::TAU;
                let ccw = *direction == ArcDirection::CounterClockwise;
                let mut sweep = if ccw {
                    (a1 - a0).rem_euclid(tau)
                } else {
                    -(a0 - a1).rem_euclid(tau)
                };
                if sweep.abs() < 1e-9 {
                    sweep = if ccw { tau } else { -tau };
                }
                let n = ((sweep.abs() / tau) * 48.0).ceil().max(2.0) as usize;
                let mut prev = start;
                for k in 1..=n {
                    let f = k as f64 / n as f64;
                    let a = a0 + sweep * f;
                    let p = Point3::new(
                        center.x + r * a.cos(),
                        center.y + r * a.sin(),
                        start.z + (to.z - start.z) * f,
                    );
                    push(&mut out, Some(prev), p, false);
                    prev = p;
                }
                at = Some(*to);
            }
            Move::Cycle(cycle) => {
                let Some(start) = at else { continue };
                for hole in &cycle.holes {
                    let over = |z: f64| Point3::new(hole.x, hole.y, z);
                    push(&mut out, at, over(start.z), true);
                    push(&mut out, Some(over(start.z)), over(cycle.clearance), true);
                    push(
                        &mut out,
                        Some(over(cycle.clearance)),
                        over(cycle.bottom),
                        false,
                    );
                    push(&mut out, Some(over(cycle.bottom)), over(start.z), true);
                    at = Some(over(start.z));
                }
            }
            Move::Dwell { .. } => {}
        }
    }
    out
}

/// Plans every operation of a stored setup (decision 0014).
///
/// # Errors
///
/// [`CamError`] when there are no setups, no such setup, no part, or an
/// operation cannot be planned.
pub fn plan_setup(
    document: &Document,
    rebuilt: &Rebuilt,
    kernel: &dyn cndt_kernel::Kernel,
    setup_id: Option<u32>,
) -> Result<Planned, CamError> {
    let cam = document.cam();
    let entry = match setup_id {
        Some(id) => cam
            .setups
            .iter()
            .find(|e| e.id == id)
            .ok_or(CamError::UnknownSetup(id))?,
        None => cam.setups.first().ok_or(CamError::NoSetups)?,
    };
    let setup = Setup::from_def(&entry.setup, rebuilt).ok_or(CamError::NoPart)?;
    let library = ToolLibrary {
        tools: cam.tools.clone(),
    };
    let name = format!("{} {}", document.name(), entry.setup.name);
    let mut program = Program {
        name,
        number: 1000 + entry.id,
        work_offset: setup.work_offset,
        operations: Vec::new(),
    };
    let mut notes = Vec::new();
    for op in &entry.setup.operations {
        match op.operation {
            OperationDef::Chamfer {
                ref face,
                width,
                tool,
            } => {
                program.operations.push(chamfer_mill(
                    rebuilt, kernel, &setup, &library, face, width, tool,
                )?);
            }
            OperationDef::Pocket { ref floor, tool } => {
                program
                    .operations
                    .push(pocket(rebuilt, kernel, &setup, &library, floor, tool)?);
            }
            OperationDef::Contour {
                ref face,
                tool,
                depth,
            } => {
                program.operations.push(contour(
                    document, rebuilt, kernel, &setup, &library, face, tool, depth,
                )?);
            }
            OperationDef::Face { tool } => {
                program
                    .operations
                    .push(face(document, rebuilt, &setup, &library, tool)?);
            }
            OperationDef::Holes => {
                let planned = hole_program(document, rebuilt, &setup, &library, &program.name)?;
                program.operations.extend(planned.program.operations);
                notes.extend(planned.notes);
            }
        }
    }
    if program.operations.is_empty() {
        return Err(CamError::Nothing);
    }
    Ok(Planned {
        program,
        notes,
        setup: Some(setup),
    })
}

/// Mills counterbores: for each, a helix down at most 3° per turn to the
/// floor, one flat circle to finish it, then to the centre and out.
fn counterbore_op(
    number: u32,
    tool: &Tool,
    setup: &Setup,
    holes: &[&Hole],
    diameter: f64,
    depth: f64,
) -> Operation {
    let r = tool.diameter / 2.0;
    let e = round3(diameter / 2.0 - r);
    let circumference = std::f64::consts::TAU * e;
    let pitch = (circumference * RAMP_DEGREES.to_radians().tan()).min(r);
    let turns = (depth / pitch).ceil().max(1.0) as usize;
    let safe = round3(setup.safe_z);
    let mut steps = vec![
        Step::Event(Event::ToolChange {
            number,
            description: if tool.description.is_empty() {
                format!("tool {number}")
            } else {
                tool.description.clone()
            },
        }),
        Step::Event(Event::SpindleOn {
            rpm: tool.rpm,
            direction: SpindleDirection::Clockwise,
        }),
        Step::Event(Event::Coolant {
            mode: Coolant::Flood,
        }),
    ];
    for hole in holes {
        let c = hole.at;
        let start = |z: f64| Point3::new(round3(c.x + e), round3(c.y), round3(z));
        let center = |z: f64| Point3::new(round3(c.x), round3(c.y), round3(z));
        let floor = c.z - depth;
        steps.push(Step::Move(Move::Rapid { to: start(safe) }));
        steps.push(Step::Move(Move::Rapid {
            to: start(c.z + 0.5),
        }));
        steps.push(Step::Move(Move::Linear {
            to: start(c.z),
            feed: tool.feed / 2.0,
        }));
        for turn in 1..=turns {
            let z = c.z - depth * turn as f64 / turns as f64;
            steps.push(Step::Move(Move::Arc {
                to: start(z),
                center: center(z),
                direction: ArcDirection::CounterClockwise,
                feed: tool.feed / 2.0,
            }));
        }
        steps.push(Step::Move(Move::Arc {
            to: start(floor),
            center: center(floor),
            direction: ArcDirection::CounterClockwise,
            feed: tool.feed,
        }));
        steps.push(Step::Move(Move::Linear {
            to: center(floor),
            feed: tool.feed,
        }));
        steps.push(Step::Move(Move::Rapid { to: center(safe) }));
    }
    steps.push(Step::Event(Event::Coolant { mode: Coolant::Off }));
    Operation {
        name: format!("Counterbore {diameter} mm x {depth} mm"),
        steps,
    }
}

/// One tool's operation: change, spindle, coolant, then each cycle from
/// the safe height.
fn operation(
    name: &str,
    (number, tool): (u32, &Tool),
    setup: &Setup,
    cycles: Vec<(CycleKind, Vec<Point3>, f64, f64)>,
) -> Operation {
    let mut steps = vec![
        Step::Event(Event::ToolChange {
            number,
            description: if tool.description.is_empty() {
                format!("tool {number}")
            } else {
                tool.description.clone()
            },
        }),
        Step::Event(Event::SpindleOn {
            rpm: tool.rpm,
            direction: SpindleDirection::Clockwise,
        }),
        Step::Event(Event::Coolant {
            mode: Coolant::Flood,
        }),
    ];
    for (kind, holes, bottom, feed) in cycles {
        let Some(first) = holes.first().copied() else {
            continue;
        };
        let top = holes.iter().map(|h| h.z).fold(f64::MIN, f64::max);
        steps.push(Step::Move(Move::Rapid {
            to: Point3::new(first.x, first.y, setup.safe_z.max(top + setup.clearance)),
        }));
        steps.push(Step::Move(Move::Cycle(Cycle {
            kind,
            holes,
            clearance: round3(top + setup.clearance),
            bottom: round3(bottom),
            feed,
        })));
    }
    steps.push(Step::Event(Event::Coolant { mode: Coolant::Off }));
    Operation {
        name: name.to_owned(),
        steps,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use cndt_cam_core::Move;
    use cndt_kernel::OpenCascade;
    use cndt_model::cam::CamEdit;
    use cndt_model::sketch::{EntityId, Point2};
    use cndt_model::{HoleSpec, Operation as Join, Plane, default_profiles, rebuild};

    /// A 96 × 56 × 20 block with one Ø5 through hole drilled from below at
    /// model (10, 10).
    fn block_with_bottom_hole() -> Document {
        let mut d = Document::new("Block");
        let s = d.add_sketch("", Plane::Xy).unwrap();
        d.edit_sketch(s, |k| {
            let p: Vec<EntityId> = [(0.0, 0.0), (96.0, 0.0), (96.0, 56.0), (0.0, 56.0)]
                .iter()
                .map(|&(x, y)| k.add_point(Point2::new(x, y)))
                .collect::<Result<_, _>>()?;
            for i in 0..4 {
                k.add_line(p[i], p[(i + 1) % 4])?;
            }
            Ok(())
        })
        .unwrap();
        let profiles = default_profiles(&d.sketch(s).unwrap().sketch);
        d.add_feature(Feature::Extrude {
            name: "Body".into(),
            sketch: s,
            profiles,
            distance: 20.0,
            distance_expression: None,
            operation: Join::Join,
        })
        .unwrap();
        let bottom = rebuild(&d, &OpenCascade)
            .faces
            .iter()
            .flatten()
            .map(ToString::to_string)
            .find(|n| n.starts_with("f0/start/"))
            .unwrap();
        let under = d.add_sketch("", Plane::Face(bottom)).unwrap();
        // On the bottom face, sketch Y is model −Y.
        let point = d
            .edit_sketch(under, |k| k.add_point(Point2::new(10.0, -10.0)))
            .unwrap();
        d.add_feature(Feature::Hole {
            name: "Under".into(),
            sketch: under,
            points: vec![point],
            spec: HoleSpec {
                diameter: 5.0,
                depth: HoleDepth::Through,
                tip_angle: 118.0,
                counterbore: None,
                thread: None,
                fit: None,
            },
        })
        .unwrap();
        d
    }

    fn first_cycle(planned: &Planned) -> Cycle {
        planned
            .program
            .operations
            .iter()
            .flat_map(|op| &op.steps)
            .find_map(|step| match step {
                Step::Move(Move::Cycle(c)) => Some(c.clone()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn the_bottom_setup_sees_the_part_flipped() {
        let mut d = block_with_bottom_hole();
        let top = d
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Top,
                work_offset: 1,
            })
            .unwrap()
            .unwrap();
        d.edit_cam(CamEdit::AddOperation {
            setup: top,
            operation: OperationDef::Holes,
        })
        .unwrap();
        let under = d
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Bottom,
                work_offset: 2,
            })
            .unwrap()
            .unwrap();
        d.edit_cam(CamEdit::AddOperation {
            setup: under,
            operation: OperationDef::Holes,
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        // From the top the hole does not face the spindle.
        assert_eq!(
            plan_setup(&d, &r, &OpenCascade, Some(top)),
            Err(CamError::Nothing)
        );
        // From below it does: flipped about X, model y = 10 is program
        // y = 56 − 10 = 46, and the bottom face is Z0.
        let planned = plan_setup(&d, &r, &OpenCascade, Some(under)).unwrap();
        assert_eq!(planned.program.work_offset, 2);
        let spot = first_cycle(&planned);
        assert_eq!(spot.holes, vec![Point3::new(10.0, 46.0, 0.0)]);
        assert_eq!(
            plan_setup(&d, &r, &OpenCascade, Some(9)),
            Err(CamError::UnknownSetup(9))
        );
    }

    #[test]
    fn facing_covers_the_stock_and_never_plunges_into_it() {
        let mut d = block_with_bottom_hole();
        let top = d
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Top,
                work_offset: 1,
            })
            .unwrap()
            .unwrap();
        d.edit_cam(CamEdit::AddOperation {
            setup: top,
            operation: OperationDef::Face { tool: None },
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            plan_setup(&d, &r, &OpenCascade, Some(top)),
            Err(CamError::NoStock)
        );
        d.edit_cam(CamEdit::SetStock(cndt_model::cam::Stock {
            side: 2.0,
            top: 2.0,
            bottom: 3.0,
        }))
        .unwrap();
        let planned = plan_setup(&d, &r, &OpenCascade, Some(top)).unwrap();
        planned.program.validate().unwrap();
        let moves: Vec<(Point3, bool)> = planned.program.operations[0]
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Move(Move::Rapid { to }) => Some((*to, false)),
                Step::Move(Move::Linear { to, .. }) => Some((*to, true)),
                _ => None,
            })
            .collect();
        // Two 1 mm levels, ending at Z0.
        let levels: std::collections::BTreeSet<i64> = moves
            .iter()
            .filter(|(_, cut)| *cut)
            .map(|(p, _)| (p.z * 1000.0) as i64)
            .collect();
        assert_eq!(levels.into_iter().collect::<Vec<_>>(), [0, 1000]);
        // The stock is x −2..98, y −2..58. A 50 mm cutter covers it in rows
        // 35 apart, and every plunge (a feed move down) is outside it.
        let rows: std::collections::BTreeSet<i64> = moves
            .iter()
            .filter(|(_, cut)| *cut)
            .map(|(p, _)| (p.y * 1000.0) as i64)
            .collect();
        let rows: Vec<f64> = rows.into_iter().map(|y| y as f64 / 1000.0).collect();
        assert!(
            rows.first().unwrap() - 25.0 <= -2.0 && rows.last().unwrap() + 25.0 >= 58.0,
            "{rows:?}"
        );
        for pair in moves.windows(2) {
            let ((a, _), (b, cut)) = (pair[0], pair[1]);
            if cut && b.z < a.z - 1e-9 {
                assert!(b.x < -2.0 - 25.0 || b.x > 98.0 + 25.0, "plunge at {b:?}");
            }
        }
        // From below there is 3 mm to take off: three passes.
        let under = d
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Bottom,
                work_offset: 2,
            })
            .unwrap()
            .unwrap();
        d.edit_cam(CamEdit::AddOperation {
            setup: under,
            operation: OperationDef::Face { tool: None },
        })
        .unwrap();
        let planned = plan_setup(&d, &r, &OpenCascade, Some(under)).unwrap();
        assert_eq!(planned.program.operations[0].name, "Face 3 mm");
    }

    #[test]
    fn the_outside_contour_climbs_one_radius_out() {
        let mut d = block_with_bottom_hole();
        d.edit_cam(CamEdit::SetStock(cndt_model::cam::Stock {
            side: 2.0,
            top: 2.0,
            bottom: 3.0,
        }))
        .unwrap();
        let top = d
            .edit_cam(CamEdit::AddSetup {
                name: String::new(),
                side: Side::Top,
                work_offset: 1,
            })
            .unwrap()
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        let bottom = r
            .faces
            .iter()
            .flatten()
            .map(ToString::to_string)
            .find(|n| n.starts_with("f0/start/"))
            .unwrap();
        d.edit_cam(CamEdit::AddOperation {
            setup: top,
            operation: OperationDef::Contour {
                face: bottom,
                tool: None,
                depth: None,
            },
        })
        .unwrap();
        let planned = plan_setup(&d, &r, &OpenCascade, Some(top)).unwrap();
        planned.program.validate().unwrap();
        let op = &planned.program.operations[0];
        assert_eq!(op.name, "Contour 20 mm deep");
        let cuts: Vec<Point3> = op
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Move(Move::Linear { to, .. } | Move::Arc { to, .. }) => Some(*to),
                _ => None,
            })
            .collect();
        // Two 10 mm levels with the 10 mm end mill.
        let levels: std::collections::BTreeSet<i64> =
            cuts.iter().map(|p| (p.z * 1000.0) as i64).collect();
        assert_eq!(levels.into_iter().collect::<Vec<_>>(), [-20000, -10000]);
        // On the path the cutter's centre is exactly 5 mm outside the
        // 96 × 56 part (a sharp-cornered box here, so corners are R5 arcs).
        let level: Vec<Point3> = cuts
            .iter()
            .copied()
            .filter(|p| (p.z + 10.0).abs() < 1e-9)
            .collect();
        let on_path = &level[1..level.len() - 1];
        for p in on_path {
            let dx = (-p.x).max(p.x - 96.0).max(0.0);
            let dy = (-p.y).max(p.y - 56.0).max(0.0);
            assert!(
                (dx.hypot(dy) - 5.0).abs() < 1e-6,
                "{p:?} is not 5 mm outside the part"
            );
        }
        // Clockwise: the signed area of the path is negative.
        let n = on_path.len();
        let area: f64 = (0..n)
            .map(|i| on_path[i].x * on_path[(i + 1) % n].y - on_path[(i + 1) % n].x * on_path[i].y)
            .sum();
        assert!(area < 0.0, "climb milling runs clockwise");
        // Plunges (feeds downwards) only outside the 2 mm stock allowance.
        let mut last = Point3::new(0.0, 0.0, 100.0);
        for s in &op.steps {
            if let Step::Move(Move::Linear { to, .. }) = s {
                if to.z < last.z - 1e-9 {
                    assert!(
                        to.x < -2.0 - 5.0
                            || to.x > 98.0 + 5.0
                            || to.y < -2.0 - 5.0
                            || to.y > 58.0 + 5.0,
                        "plunge at {to:?}"
                    );
                }
                last = *to;
            } else if let Step::Move(Move::Rapid { to }) = s {
                last = *to;
            }
        }
    }

    #[test]
    fn no_setups_says_so() {
        let d = block_with_bottom_hole();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            plan_setup(&d, &r, &OpenCascade, None),
            Err(CamError::NoSetups)
        );
    }
}
