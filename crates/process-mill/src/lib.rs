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
    Coolant, Cycle, CycleKind, Event, Move, Operation, Program, SpindleDirection, Step,
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

    /// A model point in program coordinates.
    fn program_point(&self, p: Point3) -> Point3 {
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
}

impl fmt::Display for CamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPart => write!(f, "there is no part to machine yet"),
            Self::NoTool(what) => write!(f, "the tool library has no {what}; add one"),
            Self::Nothing => write!(f, "the part has no holes this setup can make"),
            Self::UnknownSetup(id) => write!(f, "there is no setup {id}"),
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
        if spec.counterbore.is_some() {
            notes.push(format!(
                "{name}: drilled through; the counterbores need milling, which comes next"
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
    Ok(Planned { program, notes })
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

/// Plans every operation of a stored setup (decision 0014).
///
/// # Errors
///
/// [`CamError`] when there are no setups, no such setup, no part, or an
/// operation cannot be planned.
pub fn plan_setup(
    document: &Document,
    rebuilt: &Rebuilt,
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
    Ok(Planned { program, notes })
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
        assert_eq!(plan_setup(&d, &r, Some(top)), Err(CamError::Nothing));
        // From below it does: flipped about X, model y = 10 is program
        // y = 56 − 10 = 46, and the bottom face is Z0.
        let planned = plan_setup(&d, &r, Some(under)).unwrap();
        assert_eq!(planned.program.work_offset, 2);
        let spot = first_cycle(&planned);
        assert_eq!(spot.holes, vec![Point3::new(10.0, 46.0, 0.0)]);
        assert_eq!(plan_setup(&d, &r, Some(9)), Err(CamError::UnknownSetup(9)));
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
        assert_eq!(plan_setup(&d, &r, Some(top)), Err(CamError::NoStock));
        d.edit_cam(CamEdit::SetStock(cndt_model::cam::Stock {
            side: 2.0,
            top: 2.0,
            bottom: 3.0,
        }))
        .unwrap();
        let planned = plan_setup(&d, &r, Some(top)).unwrap();
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
        let planned = plan_setup(&d, &r, Some(under)).unwrap();
        assert_eq!(planned.program.operations[0].name, "Face 3 mm");
    }

    #[test]
    fn no_setups_says_so() {
        let d = block_with_bottom_hole();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(plan_setup(&d, &r, None), Err(CamError::NoSetups));
    }
}
