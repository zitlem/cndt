// SPDX-License-Identifier: AGPL-3.0-or-later
//! Solid features, and rebuilding the part from them.
//!
//! Features are applied in timeline order. A feature that cannot be built
//! is flagged with the reason and skipped, and the rest still build, as in
//! Fusion 360's timeline. Every face of the result is named by its history
//! (decision 0012): the feature and sketch curve that made it, carried
//! through every later operation by the kernel's face history.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use cndt_geom::{BoundingBox, Point3, TriangleMesh};
use cndt_kernel::{Blend, Body, BodyProperties, BooleanOp, Curve, Kernel, Made};
use cndt_sketch::{EntityId, Geometry, Loop, ProfileKey, Sketch};
use serde::{Deserialize, Serialize};

use crate::{Document, Plane, SketchId};

/// Identifies a feature. Never reused after deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureId(pub u32);

impl fmt::Display for FeatureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "feature {}", self.0)
    }
}

/// What an extrusion does to the part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Adds material.
    Join,
    /// Removes material.
    Cut,
}

/// What a revolve turns about (decision 0020): a line of its own sketch, or
/// one of the sketch plane's two axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RevolveAxis {
    /// A line of the sketch, usually construction geometry.
    Line {
        /// The line.
        entity: EntityId,
    },
    /// The sketch plane's X axis through its origin.
    SketchX,
    /// The sketch plane's Y axis through its origin.
    SketchY,
}

/// A solid feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Feature {
    /// Turns sketch profiles about an axis in their plane (decision 0020).
    Revolve {
        /// Shown in the timeline, such as `Revolve1`.
        name: String,
        /// The sketch the profiles are in.
        sketch: SketchId,
        /// The regions to turn, by name.
        profiles: Vec<ProfileKey>,
        /// What they turn about.
        axis: RevolveAxis,
        /// Degrees, more than 0 up to 360.
        angle: f64,
        /// An expression the angle follows, such as `sweep / 2`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle_expression: Option<String>,
        /// Join or cut.
        operation: Operation,
    },
    /// Sweeps sketch profiles along the sketch plane's normal.
    Extrude {
        /// Shown in the timeline, such as `Extrude1`.
        name: String,
        /// The sketch the profiles are in.
        sketch: SketchId,
        /// The regions to sweep, by name.
        profiles: Vec<ProfileKey>,
        /// Millimetres along the plane's normal; negative goes the other way.
        distance: f64,
        /// An expression the distance follows, such as `-thickness / 2`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        distance_expression: Option<String>,
        /// Join or cut.
        operation: Operation,
    },
    /// Drills holes at sketch points, into the part along the sketch's
    /// normal: simple, counterbored, and tapped (decision 0010).
    Hole {
        /// Shown in the timeline, such as `Hole1`.
        name: String,
        /// The sketch the points are in.
        sketch: SketchId,
        /// The hole centres: points of the sketch.
        points: Vec<EntityId>,
        /// Size and type.
        spec: HoleSpec,
    },
    /// Bevels edges at 45°.
    Chamfer {
        /// Shown in the timeline, such as `Chamfer1`.
        name: String,
        /// Which edges.
        edges: Vec<EdgeTarget>,
        /// Millimetres back from the edge on each face.
        distance: f64,
    },
    /// Repeats an earlier extrude or hole in a grid along its sketch's X
    /// and Y axes. Each copy joins or cuts as the original does.
    Pattern {
        /// Shown in the timeline, such as `Pattern1`.
        name: String,
        /// The feature it repeats.
        feature: FeatureId,
        /// Copies along sketch X, counting the original.
        count_x: u32,
        /// Millimetres between them; negative goes the other way.
        spacing_x: f64,
        /// Copies along sketch Y, counting the original.
        count_y: u32,
        /// Millimetres between them.
        spacing_y: f64,
    },
    /// Rounds edges.
    Fillet {
        /// Shown in the timeline, such as `Fillet1`.
        name: String,
        /// Which edges.
        edges: Vec<EdgeTarget>,
        /// Millimetres.
        radius: f64,
    },
    /// A solid read from a STEP or IGES file, joined to the part or cut
    /// from it (decision 0017). The file's content is kept in the project, so the
    /// project opens without the original file.
    Import {
        /// Shown in the timeline, such as `Import1`.
        name: String,
        /// The original file's name, without its folder, for display.
        source: String,
        /// STEP or IGES. Left out for STEP.
        #[serde(default, skip_serializing_if = "ImportFormat::is_step")]
        format: ImportFormat,
        /// Join or cut.
        operation: Operation,
        /// The content's length in bytes, to check the stored copy.
        bytes: u64,
        /// The content's 64-bit FNV-1a hash in hex, to check the stored copy.
        hash: String,
        /// The file's content. Stored as its own entry in the project file,
        /// not in the document JSON.
        #[serde(skip)]
        data: ImportData,
    },
    /// A triangle mesh read from an STL file, shown beside the part for
    /// reference and measuring (decision 0018). It never joins the solid,
    /// and nothing machines to it. Its file is kept in the project.
    Reference {
        /// Shown in the timeline, such as `Mesh1`.
        name: String,
        /// The original file's name, without its folder, for display.
        source: String,
        /// The content's length in bytes, to check the stored copy.
        bytes: u64,
        /// The content's 64-bit FNV-1a hash in hex, to check the stored copy.
        hash: String,
        /// The file's content, stored as its own entry in the project file.
        #[serde(skip)]
        data: ImportData,
    },
}

/// The exchange format of an imported file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    /// STEP, ISO 10303-21.
    #[default]
    Step,
    /// IGES.
    Iges,
}

impl ImportFormat {
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde passes fields by reference
    const fn is_step(&self) -> bool {
        matches!(self, Self::Step)
    }

    /// The file extension its stored copy takes.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Iges => "igs",
        }
    }

    /// Recognises a file by its content: STEP starts `ISO-10303-21`, IGES
    /// has its fixed 80-column lines with the section letter `S` in column
    /// 73 of the first.
    #[must_use]
    pub fn sniff(content: &str) -> Option<Self> {
        if content.trim_start().starts_with("ISO-10303-21") {
            return Some(Self::Step);
        }
        let first = content.lines().next()?;
        (first.len() >= 73 && first.as_bytes().get(72) == Some(&b'S')).then_some(Self::Iges)
    }
}

/// An imported file's content: shared, so copying the document for undo
/// does not copy the file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportData(pub std::sync::Arc<Vec<u8>>);

impl ImportData {
    /// The content's length and hash, as an import feature records them.
    #[must_use]
    pub fn fingerprint(&self) -> (u64, String) {
        (self.0.len() as u64, format!("{:016x}", fnv1a(&self.0)))
    }
}

/// 64-bit FNV-1a.
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The most an imported file may hold: 200 MB.
pub const MAX_IMPORT_BYTES: u64 = 200 * 1024 * 1024;

/// Edges to chamfer or fillet, by the names of the faces around them
/// (decision 0012).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeTarget {
    /// Every edge around a face, as picking a face does in Fusion 360.
    Face(String),
    /// The one edge where two faces meet.
    Between(String, String),
}

/// How deep a hole goes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleDepth {
    /// To this depth in millimetres, measured to the end of the full
    /// diameter (the drill point goes further).
    Blind(f64),
    /// Through the whole part.
    Through,
}

/// A counterbore at the top of a hole.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counterbore {
    /// Millimetres; larger than the hole.
    pub diameter: f64,
    /// Millimetres.
    pub depth: f64,
}

/// A thread cut into a hole. The hole itself is modelled at its drill
/// diameter; the thread is carried to CAM, which picks the tap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thread {
    /// The thread size, such as `M6x1.0` (see [`crate::threads`]).
    pub size: String,
    /// Full-thread depth in millimetres.
    pub depth: f64,
}

/// The size and type of a hole.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoleSpec {
    /// The drilled diameter in millimetres (the tap drill for a tapped hole).
    pub diameter: f64,
    /// Blind or through.
    pub depth: HoleDepth,
    /// The drill point's included angle in degrees; 180 is a flat bottom.
    pub tip_angle: f64,
    /// An optional counterbore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterbore: Option<Counterbore>,
    /// An optional thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<Thread>,
    /// An optional fit, such as `H7`, for CAM to finish to (for example by
    /// reaming).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
}

impl HoleSpec {
    fn check(&self) -> Result<(), String> {
        let positive = |what: &str, v: f64| {
            if v.is_finite() && v > 0.0 {
                Ok(())
            } else {
                Err(format!("the {what} must be more than zero, not {v}"))
            }
        };
        positive("hole diameter", self.diameter)?;
        if let HoleDepth::Blind(depth) = self.depth {
            positive("hole depth", depth)?;
        }
        if !(self.tip_angle.is_finite() && self.tip_angle > 0.0 && self.tip_angle <= 180.0) {
            return Err(format!(
                "the drill point angle must be more than 0° and at most 180°, not {}",
                self.tip_angle
            ));
        }
        if let Some(cb) = self.counterbore {
            positive("counterbore depth", cb.depth)?;
            if !(cb.diameter.is_finite() && cb.diameter > self.diameter) {
                return Err(format!(
                    "the counterbore ({} mm) must be wider than the hole ({} mm)",
                    cb.diameter, self.diameter
                ));
            }
            if let HoleDepth::Blind(depth) = self.depth
                && cb.depth >= depth
            {
                return Err(String::from(
                    "the counterbore must be shallower than the hole",
                ));
            }
        }
        if let Some(thread) = &self.thread {
            let size = crate::threads::thread(&thread.size).ok_or_else(|| {
                format!(
                    "unknown thread {:?}; use a metric coarse size such as M6x1.0",
                    thread.size
                )
            })?;
            positive("thread depth", thread.depth)?;
            if size.major <= self.diameter {
                return Err(format!(
                    "a {} thread needs a hole smaller than {} mm",
                    size.designation, size.major
                ));
            }
            if let HoleDepth::Blind(depth) = self.depth
                && thread.depth > depth
            {
                return Err(format!(
                    "the thread ({} mm) is deeper than the hole ({depth} mm)",
                    thread.depth
                ));
            }
        }
        if let Some(fit) = &self.fit
            && (fit.is_empty() || fit.len() > 20 || fit.chars().any(char::is_control))
        {
            return Err(format!("{fit:?} is not a fit, such as H7"));
        }
        Ok(())
    }
}

impl Feature {
    /// The timeline name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Extrude { name, .. }
            | Self::Revolve { name, .. }
            | Self::Hole { name, .. }
            | Self::Chamfer { name, .. }
            | Self::Pattern { name, .. }
            | Self::Fillet { name, .. }
            | Self::Import { name, .. }
            | Self::Reference { name, .. } => name,
        }
    }

    /// The sketch it uses, if any.
    #[must_use]
    pub const fn sketch(&self) -> Option<SketchId> {
        match self {
            Self::Extrude { sketch, .. }
            | Self::Revolve { sketch, .. }
            | Self::Hole { sketch, .. } => Some(*sketch),
            Self::Chamfer { .. }
            | Self::Fillet { .. }
            | Self::Pattern { .. }
            | Self::Import { .. }
            | Self::Reference { .. } => None,
        }
    }

    /// Checks the values the editing commands must keep.
    pub(crate) fn check(&self) -> Result<(), String> {
        match self {
            Self::Import {
                source,
                bytes,
                hash,
                data,
                ..
            }
            | Self::Reference {
                source,
                bytes,
                hash,
                data,
                ..
            } => {
                if data.0.is_empty() {
                    return Err(format!("the imported file {source} has no content"));
                }
                if *bytes > MAX_IMPORT_BYTES {
                    return Err(format!(
                        "{source} is {bytes} bytes; the most an import may be is {MAX_IMPORT_BYTES}"
                    ));
                }
                if data.fingerprint() != (*bytes, hash.clone()) {
                    return Err(format!(
                        "the stored copy of {source} does not match its recorded length and hash"
                    ));
                }
                Ok(())
            }
            Self::Extrude {
                distance, profiles, ..
            } => {
                if !(distance.is_finite() && distance.abs() > 1e-9) {
                    return Err(format!(
                        "the extrude distance must be a non-zero number, not {distance}"
                    ));
                }
                if profiles.is_empty() {
                    return Err(String::from("pick at least one profile to extrude"));
                }
                Ok(())
            }
            Self::Revolve {
                angle, profiles, ..
            } => {
                if !(angle.is_finite() && *angle > 0.0 && *angle <= 360.0) {
                    return Err(format!(
                        "the revolve angle must be more than 0° and at most 360°, not {angle}"
                    ));
                }
                if profiles.is_empty() {
                    return Err(String::from("pick at least one profile to revolve"));
                }
                Ok(())
            }
            Self::Hole { points, spec, .. } => {
                if points.is_empty() {
                    return Err(String::from("pick at least one point to put a hole at"));
                }
                spec.check()
            }
            Self::Pattern {
                count_x,
                spacing_x,
                count_y,
                spacing_y,
                ..
            } => {
                if *count_x == 0 || *count_y == 0 {
                    return Err(String::from(
                        "a pattern needs at least one copy in each direction",
                    ));
                }
                let total = u64::from(*count_x) * u64::from(*count_y);
                if total < 2 {
                    return Err(String::from("a pattern needs more than one copy"));
                }
                if total > 500 {
                    return Err(format!("{total} copies is too many; the most is 500"));
                }
                for (count, spacing, axis) in [(count_x, spacing_x, "X"), (count_y, spacing_y, "Y")]
                {
                    if *count > 1 && !(spacing.is_finite() && spacing.abs() > 1e-9) {
                        return Err(format!(
                            "the {axis} spacing must be a non-zero number, not {spacing}"
                        ));
                    }
                }
                Ok(())
            }
            Self::Chamfer {
                edges,
                distance: size,
                ..
            }
            | Self::Fillet {
                edges,
                radius: size,
                ..
            } => {
                let what = if matches!(self, Self::Chamfer { .. }) {
                    "chamfer distance"
                } else {
                    "fillet radius"
                };
                if !(size.is_finite() && *size > 0.0) {
                    return Err(format!("the {what} must be more than zero, not {size}"));
                }
                if edges.is_empty() {
                    return Err(String::from("pick at least one edge or face"));
                }
                let bad =
                    |t: &String| t.is_empty() || t.len() > 200 || t.chars().any(char::is_control);
                if edges.iter().any(|e| match e {
                    EdgeTarget::Face(f) => bad(f),
                    EdgeTarget::Between(a, b) => bad(a) || bad(b),
                }) {
                    return Err(String::from("an edge is named by a damaged face name"));
                }
                Ok(())
            }
        }
    }
}

/// A feature in the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureEntry {
    /// Its id.
    pub id: FeatureId,
    /// The feature.
    pub feature: Feature,
}

/// Where a face came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FaceOrigin {
    /// Swept from a sketch curve.
    Side {
        /// The line, arc or circle.
        curve: EntityId,
    },
    /// The cap at the sketch plane.
    Start {
        /// The profile it closes.
        profile: ProfileKey,
    },
    /// The far cap.
    End {
        /// The profile it closes.
        profile: ProfileKey,
    },
    /// The face a chamfer or fillet made along an edge, named by the faces
    /// the edge joined.
    Blend {
        /// The two faces' names, in order.
        between: (String, String),
    },
    /// A face of a pattern's copy: the original face's name, and the copy's
    /// place in the grid.
    Copy {
        /// Column and row; the original is (0, 0).
        at: (u32, u32),
        /// The original face's name.
        original: String,
    },
    /// A surface of a hole.
    Hole {
        /// The sketch point at its centre.
        point: EntityId,
        /// Which surface.
        surface: HoleSurface,
    },
    /// A face of an imported solid, by its number in the file's solid.
    Imported {
        /// The face number, the same for the same file.
        face: u32,
    },
}

/// The surfaces of a hole, from the top down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleSurface {
    /// The counterbore's wall.
    CounterboreWall,
    /// The counterbore's floor.
    CounterboreFloor,
    /// The hole's wall.
    Wall,
    /// The cone left by the drill point.
    Tip,
    /// A flat bottom.
    Floor,
}

impl HoleSurface {
    const fn token(self) -> &'static str {
        match self {
            Self::CounterboreWall => "counterbore_wall",
            Self::CounterboreFloor => "counterbore_floor",
            Self::Wall => "wall",
            Self::Tip => "tip",
            Self::Floor => "floor",
        }
    }
}

/// A face's history name: the feature that made it and how.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct FaceName {
    /// The feature that created it.
    pub feature: FeatureId,
    /// How.
    #[serde(flatten)]
    pub origin: FaceOrigin,
}

impl fmt::Display for FaceName {
    /// The persistent token: `f<feature>/side/e<curve>`,
    /// `f<feature>/start/<profile>` or `f<feature>/end/<profile>`, where a
    /// profile is its curve ids joined by `-`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = |k: &ProfileKey| {
            k.0.iter()
                .map(|e| e.0.to_string())
                .collect::<Vec<_>>()
                .join("-")
        };
        match &self.origin {
            FaceOrigin::Side { curve } => write!(f, "f{}/side/e{}", self.feature.0, curve.0),
            FaceOrigin::Start { profile } => {
                write!(f, "f{}/start/{}", self.feature.0, key(profile))
            }
            FaceOrigin::End { profile } => write!(f, "f{}/end/{}", self.feature.0, key(profile)),
            FaceOrigin::Copy { at, original } => {
                write!(f, "f{}/at{},{}/{original}", self.feature.0, at.0, at.1)
            }
            FaceOrigin::Blend { between } => {
                write!(f, "f{}/blend/{}+{}", self.feature.0, between.0, between.1)
            }
            FaceOrigin::Imported { face } => write!(f, "f{}/import/{face}", self.feature.0),
            FaceOrigin::Hole { point, surface } => {
                write!(
                    f,
                    "f{}/hole/e{}/{}",
                    self.feature.0,
                    point.0,
                    surface.token()
                )
            }
        }
    }
}

/// How a feature built.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FeatureStatus {
    /// It built.
    Ok,
    /// It could not be built and was skipped. Says why and what to do.
    Failed {
        /// The reason.
        reason: String,
    },
}

/// The part as rebuilt from the timeline.
#[derive(Debug, Clone, Default)]
pub struct Rebuilt {
    /// The solid, once any feature has made one.
    pub body: Option<Body>,
    /// The history names of each face of `body`, by face number. A face
    /// merged from several has several names.
    pub faces: Vec<BTreeSet<FaceName>>,
    /// Each feature's outcome.
    pub status: BTreeMap<FeatureId, FeatureStatus>,
    /// Size of the body.
    pub properties: Option<BodyProperties>,
    /// Where each sketch sits: for a sketch a feature uses, as placed for
    /// the first such feature; otherwise on the finished part.
    pub frames: BTreeMap<SketchId, FrameStatus>,
    /// Each reference mesh that read, by its feature (decision 0018).
    pub references: BTreeMap<FeatureId, ReferenceMesh>,
}

/// A reference mesh as rebuilt: its triangles and its size.
#[derive(Debug, Clone)]
pub struct ReferenceMesh {
    /// The triangles, shared.
    pub mesh: std::sync::Arc<TriangleMesh>,
    /// The box around it.
    pub bounds: BoundingBox,
}

/// The last few meshes read, by the content they were read from (the same
/// shared buffer), so an edit elsewhere does not read a large file again.
static MESHES: std::sync::Mutex<Vec<(std::sync::Arc<Vec<u8>>, ReferenceMesh)>> =
    std::sync::Mutex::new(Vec::new());

/// Reads a reference mesh, or takes it from [`MESHES`].
fn reference_mesh(data: &ImportData) -> Result<ReferenceMesh, String> {
    if let Ok(cache) = MESHES.lock()
        && let Some((_, mesh)) = cache
            .iter()
            .find(|(bytes, _)| std::sync::Arc::ptr_eq(bytes, &data.0))
    {
        return Ok(mesh.clone());
    }
    let mesh = cndt_geom::stl::read(&data.0)?;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for corner in mesh.positions.as_chunks::<3>().0 {
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(corner[axis]));
            max[axis] = max[axis].max(f64::from(corner[axis]));
        }
    }
    let reference = ReferenceMesh {
        mesh: std::sync::Arc::new(mesh),
        bounds: BoundingBox {
            min: Point3::new(min[0], min[1], min[2]),
            max: Point3::new(max[0], max[1], max[2]),
        },
    };
    if let Ok(mut cache) = MESHES.lock() {
        if cache.len() >= 4 {
            cache.remove(0);
        }
        cache.push((data.0.clone(), reference.clone()));
    }
    Ok(reference)
}

/// Faces named after an operation: result face → names.
type Names = Vec<BTreeSet<FaceName>>;

/// Carries names through an operation's history.
fn carry(made: &Made, sources: &dyn Fn(u32) -> Option<BTreeSet<FaceName>>) -> Names {
    let count = made
        .history
        .iter()
        .map(|(_, f)| *f as usize + 1)
        .max()
        .unwrap_or(0);
    let mut names = vec![BTreeSet::new(); count];
    for &(source, face) in &made.history {
        if let (Some(slot), Some(set)) = (names.get_mut(face as usize), sources(source)) {
            slot.extend(set);
        }
    }
    names
}

/// Where a sketch sits in the model: its origin, axes and normal, all in
/// model coordinates. Counter-clockwise in the sketch is counter-clockwise
/// about the normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Frame {
    /// Sketch (0, 0).
    pub origin: Point3,
    /// Sketch +X, a unit vector.
    pub x: [f64; 3],
    /// Sketch +Y, a unit vector.
    pub y: [f64; 3],
    /// The sketch's normal: out of the part for a face sketch.
    pub normal: [f64; 3],
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(a, a).sqrt();
    (length > 1e-9).then(|| [a[0] / length, a[1] / length, a[2] / length])
}

impl Frame {
    /// The frame of an origin plane, or `None` for a face.
    #[must_use]
    pub fn of_origin_plane(plane: &Plane) -> Option<Self> {
        let o = Point3::new(0.0, 0.0, 0.0);
        let (x, y) = match plane {
            Plane::Xy => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            Plane::Xz => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Plane::Yz => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            Plane::Face(_) => return None,
        };
        Some(Self {
            origin: o,
            x,
            y,
            normal: cross(x, y),
        })
    }

    /// The frame of a flat face through `point` with outward `normal`. The
    /// origin is the model origin projected onto the face, and X is the
    /// model X axis projected onto it (model Y when X is along the normal),
    /// so a top face's sketch coordinates are the model's X and Y.
    #[must_use]
    pub fn on_face(point: Point3, normal: [f64; 3]) -> Option<Self> {
        let n = unit(normal)?;
        let p = [point.x, point.y, point.z];
        let d = dot(p, n);
        let origin = Point3::new(n[0] * d, n[1] * d, n[2] * d);
        let project = |a: [f64; 3]| {
            let k = dot(a, n);
            unit([a[0] - k * n[0], a[1] - k * n[1], a[2] - k * n[2]])
        };
        let x = project([1.0, 0.0, 0.0]).or_else(|| project([0.0, 1.0, 0.0]))?;
        Some(Self {
            origin,
            x,
            y: cross(n, x),
            normal: n,
        })
    }

    /// A sketch point in model coordinates.
    #[must_use]
    pub fn at(&self, x: f64, y: f64) -> Point3 {
        let o = self.origin;
        Point3::new(
            o.x + x * self.x[0] + y * self.y[0],
            o.y + x * self.x[1] + y * self.y[1],
            o.z + x * self.x[2] + y * self.y[2],
        )
    }
}

/// Where a sketch could be placed, or why not.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FrameStatus {
    /// Placed.
    Ok {
        /// The frame.
        frame: Frame,
    },
    /// Its face could not be found; says why.
    Failed {
        /// The reason.
        reason: String,
    },
}

/// Finds a sketch's frame on the part as it is at this point of the
/// timeline. A face name must match exactly one flat face: none or several
/// is reported, never guessed (decision 0012).
fn resolve(
    plane: &Plane,
    part: Option<&(Body, Names)>,
    kernel: &dyn Kernel,
) -> Result<Frame, String> {
    if let Some(frame) = Frame::of_origin_plane(plane) {
        return Ok(frame);
    }
    let Plane::Face(token) = plane else {
        return Err(String::from("unknown plane"));
    };
    let Some((body, names)) = part else {
        return Err(format!(
            "its face ({token}) does not exist at this point of the timeline"
        ));
    };
    let matches: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.iter().any(|name| name.to_string() == *token))
        .map(|(i, _)| i)
        .collect();
    let face = match matches.as_slice() {
        [one] => u32::try_from(*one).map_err(|_| String::from("too many faces"))?,
        [] => {
            return Err(format!(
                "the face it is on ({token}) no longer exists; edit the sketch and pick a face again"
            ));
        }
        _ => {
            return Err(format!(
                "the face it is on ({token}) is now split into {} faces; edit the sketch and pick one",
                matches.len()
            ));
        }
    };
    let (point, normal) = kernel.face_plane(body, face).map_err(|e| e.to_string())?;
    Frame::on_face(point, normal).ok_or_else(|| String::from("the face has no usable normal"))
}

/// A loop as kernel curves, with the sketch curve of each.
fn loop_curves(sketch: &Sketch, frame: &Frame, l: &Loop) -> Result<Vec<(Curve, EntityId)>, String> {
    let point = |id: EntityId| {
        sketch
            .point(id)
            .map(|p| frame.at(p.x, p.y))
            .ok_or_else(|| format!("{id} is missing"))
    };
    let mut out = Vec::with_capacity(l.curves.len());
    for c in &l.curves {
        let curve = match sketch.entity(c.entity).map(|e| &e.geometry) {
            Some(Geometry::Line { start, end }) => {
                let (a, b) = if c.reversed {
                    (*end, *start)
                } else {
                    (*start, *end)
                };
                Curve::Line {
                    start: point(a)?,
                    end: point(b)?,
                }
            }
            Some(Geometry::Arc { center, start, end }) => Curve::Arc {
                center: point(*center)?,
                start: point(*start)?,
                end: point(*end)?,
            },
            Some(Geometry::Circle { center, radius }) => Curve::Circle {
                center: point(*center)?,
                radius: *radius,
            },
            _ => return Err(format!("{} is not a curve", c.entity)),
        };
        out.push((curve, c.entity));
    }
    Ok(out)
}

/// The profiles a new extrusion uses when none are picked: every region
/// that is material when the sketch is read as a part outline.
#[must_use]
pub fn default_profiles(sketch: &Sketch) -> Vec<ProfileKey> {
    sketch
        .profiles()
        .profiles
        .into_iter()
        .filter(|p| p.depth % 2 == 0)
        .map(|p| p.key)
        .collect()
}

/// Builds one extrude's tool body and its face names.
fn extrude_tool(
    document: &Document,
    kernel: &dyn Kernel,
    id: FeatureId,
    feature: &Feature,
    frame: &Frame,
) -> Result<(Body, Names), String> {
    let Feature::Extrude {
        sketch,
        profiles,
        distance,
        ..
    } = feature
    else {
        return Err(String::from("not an extrusion"));
    };
    let record = document
        .sketch(*sketch)
        .ok_or_else(|| format!("its sketch ({sketch}) has been deleted"))?;
    let found = record.sketch.profiles();
    let mut tool: Option<(Body, Names)> = None;
    for key in profiles {
        let profile = found.get(key).ok_or_else(|| {
            format!(
                "a profile it uses is no longer closed in {}; edit the extrude and pick the profiles again",
                record.name
            )
        })?;
        let mut loops = Vec::new();
        let mut origins = Vec::new();
        for l in std::iter::once(&profile.outer).chain(&profile.holes) {
            let curves = loop_curves(&record.sketch, frame, l)?;
            origins.extend(curves.iter().map(|(_, e)| *e));
            loops.push(curves.into_iter().map(|(c, _)| c).collect::<Vec<_>>());
        }
        let n = u32::try_from(origins.len()).unwrap_or(u32::MAX);
        let made = kernel
            .extrude(&loops, frame.normal, *distance)
            .map_err(|e| format!("the extrusion failed: {e}"))?;
        let name = |origin| {
            BTreeSet::from([FaceName {
                feature: id,
                origin,
            }])
        };
        let names = carry(&made, &|source| {
            let origin = if source < n {
                FaceOrigin::Side {
                    curve: *origins.get(source as usize)?,
                }
            } else if source == n {
                FaceOrigin::Start {
                    profile: key.clone(),
                }
            } else {
                FaceOrigin::End {
                    profile: key.clone(),
                }
            };
            Some(name(origin))
        });
        tool = Some(match tool {
            None => (made.body, names),
            Some((body, before)) => {
                combine(kernel, &body, &before, &made.body, &names, BooleanOp::Fuse)?
            }
        });
    }
    tool.ok_or_else(|| String::from("pick at least one profile to extrude"))
}

/// The axis in sketch coordinates: a point on it and its direction.
fn axis_in_sketch(sketch: &Sketch, axis: RevolveAxis) -> Result<([f64; 2], [f64; 2]), String> {
    match axis {
        RevolveAxis::SketchX => Ok(([0.0, 0.0], [1.0, 0.0])),
        RevolveAxis::SketchY => Ok(([0.0, 0.0], [0.0, 1.0])),
        RevolveAxis::Line { entity } => {
            let Some(Geometry::Line { start, end }) = sketch.entity(entity).map(|e| &e.geometry)
            else {
                return Err(format!("{entity} is not a line, so it cannot be the axis"));
            };
            let (a, b) = match (sketch.point(*start), sketch.point(*end)) {
                (Some(a), Some(b)) => (a, b),
                _ => return Err(format!("the axis {entity} has lost an end point")),
            };
            let d = [b.x - a.x, b.y - a.y];
            if d[0].hypot(d[1]) < 1e-9 {
                return Err(format!("the axis {entity} has no length"));
            }
            Ok(([a.x, a.y], d))
        }
    }
}

/// Points along a sketch curve, enough to tell which side of a line it is on.
fn curve_points(sketch: &Sketch, entity: EntityId) -> Vec<[f64; 2]> {
    match sketch.entity(entity).map(|e| &e.geometry) {
        Some(Geometry::Line { start, end }) => [start, end]
            .iter()
            .filter_map(|p| sketch.point(**p))
            .map(|p| [p.x, p.y])
            .collect(),
        Some(Geometry::Arc { center, start, end }) => {
            let (Some(c), Some(s), Some(e)) = (
                sketch.point(*center),
                sketch.point(*start),
                sketch.point(*end),
            ) else {
                return Vec::new();
            };
            let r = c.distance(s);
            let from = (s.y - c.y).atan2(s.x - c.x);
            let to = (e.y - c.y).atan2(e.x - c.x);
            let sweep = (to - from).rem_euclid(std::f64::consts::TAU);
            (0..=64)
                .map(|k| {
                    let a = from + sweep * f64::from(k) / 64.0;
                    [c.x + r * a.cos(), c.y + r * a.sin()]
                })
                .collect()
        }
        Some(Geometry::Circle { center, radius }) => sketch
            .point(*center)
            .map(|c| {
                (0..64)
                    .map(|k| {
                        let a = std::f64::consts::TAU * f64::from(k) / 64.0;
                        [c.x + radius * a.cos(), c.y + radius * a.sin()]
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Builds one revolve's tool body and its face names. The faces are named as
/// an extrude's are: by the feature and the sketch curve that swept them,
/// with start and end caps for a partial turn.
fn revolve_tool(
    document: &Document,
    kernel: &dyn Kernel,
    id: FeatureId,
    feature: &Feature,
    frame: &Frame,
) -> Result<(Body, Names), String> {
    let Feature::Revolve {
        sketch,
        profiles,
        axis,
        angle,
        ..
    } = feature
    else {
        return Err(String::from("not a revolve"));
    };
    let record = document
        .sketch(*sketch)
        .ok_or_else(|| format!("its sketch ({sketch}) has been deleted"))?;
    let (through, direction) = axis_in_sketch(&record.sketch, *axis)?;
    let length = direction[0].hypot(direction[1]);
    let side = |p: [f64; 2]| {
        (direction[0] * (p[1] - through[1]) - direction[1] * (p[0] - through[0])) / length
    };
    let origin = frame.at(through[0], through[1]);
    let world_direction = [
        frame.x[0] * direction[0] + frame.y[0] * direction[1],
        frame.x[1] * direction[0] + frame.y[1] * direction[1],
        frame.x[2] * direction[0] + frame.y[2] * direction[1],
    ];
    let found = record.sketch.profiles();
    let mut tool: Option<(Body, Names)> = None;
    for key in profiles {
        let profile = found.get(key).ok_or_else(|| {
            format!(
                "a profile it uses is no longer closed in {}; edit the revolve and pick the profiles again",
                record.name
            )
        })?;
        let mut loops = Vec::new();
        let mut origins = Vec::new();
        let (mut low, mut high) = (0.0_f64, 0.0_f64);
        for l in std::iter::once(&profile.outer).chain(&profile.holes) {
            let curves = loop_curves(&record.sketch, frame, l)?;
            for (_, entity) in &curves {
                for p in curve_points(&record.sketch, *entity) {
                    let d = side(p);
                    low = low.min(d);
                    high = high.max(d);
                }
            }
            origins.extend(curves.iter().map(|(_, e)| *e));
            loops.push(curves.into_iter().map(|(c, _)| c).collect::<Vec<_>>());
        }
        if low < -1e-6 && high > 1e-6 {
            return Err(format!(
                "a profile in {} crosses the axis, which would fold the solid into itself; \
                 move the profile or the axis so the profile stays on one side",
                record.name
            ));
        }
        let n = u32::try_from(origins.len()).unwrap_or(u32::MAX);
        let made = kernel
            .revolve_profile(&loops, frame.normal, origin, world_direction, *angle)
            .map_err(|e| format!("the revolve failed: {e}"))?;
        let name = |origin| {
            BTreeSet::from([FaceName {
                feature: id,
                origin,
            }])
        };
        let names = carry(&made, &|source| {
            let origin = if source < n {
                FaceOrigin::Side {
                    curve: *origins.get(source as usize)?,
                }
            } else if source == n {
                FaceOrigin::Start {
                    profile: key.clone(),
                }
            } else {
                FaceOrigin::End {
                    profile: key.clone(),
                }
            };
            Some(name(origin))
        });
        tool = Some(match tool {
            None => (made.body, names),
            Some((body, before)) => {
                combine(kernel, &body, &before, &made.body, &names, BooleanOp::Fuse)?
            }
        });
    }
    tool.ok_or_else(|| String::from("pick at least one profile to revolve"))
}

/// Fuses or cuts, carrying both sides' names.
fn combine(
    kernel: &dyn Kernel,
    a: &Body,
    a_names: &Names,
    b: &Body,
    b_names: &Names,
    op: BooleanOp,
) -> Result<(Body, Names), String> {
    let made = kernel.boolean(a, b, op).map_err(|e| e.to_string())?;
    let split = u32::try_from(a_names.len()).unwrap_or(u32::MAX);
    let names = carry(&made, &|source| {
        if source < split {
            a_names.get(source as usize).cloned()
        } else {
            b_names.get((source - split) as usize).cloned()
        }
    });
    Ok((made.body, names))
}

/// How far a through hole must reach: past any point of the part.
fn reach(kernel: &dyn Kernel, part: &Body) -> f64 {
    kernel.properties(part).map_or(10_000.0, |p| {
        let [x, y, z] = p.bounds.size();
        let far = [p.bounds.min, p.bounds.max]
            .iter()
            .map(|c| c.x.abs().max(c.y.abs()).max(c.z.abs()))
            .fold(0.0, f64::max);
        2.0 * (x.hypot(y).hypot(z) + far) + 1.0
    })
}

/// Builds the cutting tool for a hole feature: one revolved half-section per
/// point, fused, with every face named.
fn hole_tool(
    document: &Document,
    kernel: &dyn Kernel,
    id: FeatureId,
    feature: &Feature,
    frame: &Frame,
    part: &Body,
) -> Result<(Body, Names), String> {
    let Feature::Hole {
        sketch,
        points,
        spec,
        ..
    } = feature
    else {
        return Err(String::from("not a hole"));
    };
    let record = document
        .sketch(*sketch)
        .ok_or_else(|| format!("its sketch ({sketch}) has been deleted"))?;
    let r = spec.diameter / 2.0;
    let depth = match spec.depth {
        HoleDepth::Blind(d) => d,
        HoleDepth::Through => reach(kernel, part),
    };
    // The half-section, as (radius, depth into the part), from the axis at
    // the top, round the outside, back up the axis; with each edge's face.
    let mut section: Vec<(f64, f64)> = vec![(0.0, 0.0)];
    let mut surfaces: Vec<Option<HoleSurface>> = Vec::new();
    if let Some(cb) = spec.counterbore {
        let rc = cb.diameter / 2.0;
        section.extend([(rc, 0.0), (rc, cb.depth), (r, cb.depth)]);
        surfaces.extend([
            None,
            Some(HoleSurface::CounterboreWall),
            Some(HoleSurface::CounterboreFloor),
        ]);
    } else {
        section.push((r, 0.0));
        surfaces.push(None);
    }
    section.push((r, depth));
    surfaces.push(Some(HoleSurface::Wall));
    let pointed = matches!(spec.depth, HoleDepth::Blind(_)) && spec.tip_angle < 180.0 - 1e-9;
    if pointed {
        section.push((0.0, depth + r / (spec.tip_angle.to_radians() / 2.0).tan()));
        surfaces.push(Some(HoleSurface::Tip));
    } else {
        section.push((0.0, depth));
        surfaces.push(Some(HoleSurface::Floor));
    }
    // The closing edge runs up the axis and sweeps nothing.
    surfaces.push(None);
    let n = frame.normal;
    let mut tool: Option<(Body, Names)> = None;
    for &point in points {
        let at = record
            .sketch
            .point(point)
            .ok_or_else(|| format!("hole point {point} is no longer in {}", record.name))?;
        let c = frame.at(at.x, at.y);
        let model = |(radius, down): (f64, f64)| {
            Point3::new(
                c.x + radius * frame.x[0] - down * n[0],
                c.y + radius * frame.x[1] - down * n[1],
                c.z + radius * frame.x[2] - down * n[2],
            )
        };
        let profile: Vec<Point3> = section.iter().copied().map(model).collect();
        let made = kernel
            .revolve(&profile, c, [-n[0], -n[1], -n[2]])
            .map_err(|e| format!("the hole at {point} could not be made: {e}"))?;
        let names = carry(&made, &|source| {
            let surface = (*surfaces.get(source as usize)?)?;
            Some(BTreeSet::from([FaceName {
                feature: id,
                origin: FaceOrigin::Hole { point, surface },
            }]))
        });
        tool = Some(match tool {
            None => (made.body, names),
            Some((body, before)) => {
                combine(kernel, &body, &before, &made.body, &names, BooleanOp::Fuse)?
            }
        });
    }
    tool.ok_or_else(|| String::from("pick at least one point to put a hole at"))
}

/// The one face named `token`, or why not.
fn unique_face(names: &Names, token: &str) -> Result<u32, String> {
    let found: Vec<usize> = names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.iter().any(|name| name.to_string() == token))
        .map(|(i, _)| i)
        .collect();
    match found.as_slice() {
        [one] => u32::try_from(*one).map_err(|_| String::from("too many faces")),
        [] => Err(format!(
            "the face {token} no longer exists; edit the feature and pick again"
        )),
        _ => Err(format!(
            "the face {token} is now split into {} faces; edit the feature and pick again",
            found.len()
        )),
    }
}

/// Chamfers or fillets the part, naming each new face by the faces its edge
/// joined.
fn blend_feature(
    kernel: &dyn Kernel,
    id: FeatureId,
    feature: &Feature,
    body: &Body,
    names: &Names,
) -> Result<(Body, Names), String> {
    let (targets, blend) = match feature {
        Feature::Chamfer {
            edges, distance, ..
        } => (edges, Blend::Chamfer(*distance)),
        Feature::Fillet { edges, radius, .. } => (edges, Blend::Fillet(*radius)),
        _ => return Err(String::from("not a chamfer or fillet")),
    };
    let joins = kernel.edges(body).map_err(|e| e.to_string())?;
    let mut picked: BTreeSet<u32> = BTreeSet::new();
    for target in targets {
        match target {
            EdgeTarget::Face(token) => {
                let face = unique_face(names, token)?;
                for (e, (a, b)) in joins.iter().enumerate() {
                    if *a == face || *b == face {
                        picked
                            .insert(u32::try_from(e).map_err(|_| String::from("too many edges"))?);
                    }
                }
            }
            EdgeTarget::Between(first, second) => {
                let (fa, fb) = (unique_face(names, first)?, unique_face(names, second)?);
                let found: Vec<usize> = joins
                    .iter()
                    .enumerate()
                    .filter(|(_, (a, b))| (*a == fa && *b == fb) || (*a == fb && *b == fa))
                    .map(|(e, _)| e)
                    .collect();
                match found.as_slice() {
                    [one] => {
                        picked.insert(
                            u32::try_from(*one).map_err(|_| String::from("too many edges"))?,
                        );
                    }
                    [] => {
                        return Err(format!(
                            "{first} and {second} no longer meet at an edge; edit the feature and pick again"
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "{first} and {second} now meet at {} edges; edit the feature and pick one",
                            found.len()
                        ));
                    }
                }
            }
        }
    }
    let edges: Vec<u32> = picked.into_iter().collect();
    let made = kernel
        .blend(body, &edges, blend)
        .map_err(|e| e.to_string())?;
    let split = u32::try_from(names.len()).unwrap_or(u32::MAX);
    // A face's first name stands for it in the new face's name.
    let label = |face: u32| {
        names
            .get(face as usize)
            .and_then(|n| n.iter().next())
            .map(ToString::to_string)
    };
    let carried = carry(&made, &|source| {
        if source < split {
            names.get(source as usize).cloned()
        } else {
            let edge = *edges.get((source - split) as usize)?;
            let (a, b) = *joins.get(edge as usize)?;
            let (mut x, mut y) = (label(a)?, label(b)?);
            if y < x {
                std::mem::swap(&mut x, &mut y);
            }
            Some(BTreeSet::from([FaceName {
                feature: id,
                origin: FaceOrigin::Blend { between: (x, y) },
            }]))
        }
    });
    Ok((made.body, carried))
}

/// What an extrude or hole added or removed, kept for patterns.
struct Tool {
    body: Body,
    names: Names,
    operation: Operation,
    frame: Frame,
}

/// Builds a pattern's copies of an earlier feature's tool, fused into one
/// body with every face named by its original and its place in the grid.
fn pattern_tool(
    kernel: &dyn Kernel,
    id: FeatureId,
    feature: &Feature,
    tools: &BTreeMap<FeatureId, Tool>,
) -> Result<(Body, Names, Operation), String> {
    let Feature::Pattern {
        feature: source,
        count_x,
        spacing_x,
        count_y,
        spacing_y,
        ..
    } = feature
    else {
        return Err(String::from("not a pattern"));
    };
    let original = tools.get(source).ok_or_else(|| {
        format!("the feature it repeats ({source}) did not build, comes later, or is not an extrude or hole")
    })?;
    let mut tool: Option<(Body, Names)> = None;
    for j in 0..*count_y {
        for i in 0..*count_x {
            if i == 0 && j == 0 {
                continue;
            }
            let (di, dj) = (f64::from(i) * spacing_x, f64::from(j) * spacing_y);
            let x = original.frame.x;
            let y = original.frame.y;
            let offset = [
                di * x[0] + dj * y[0],
                di * x[1] + dj * y[1],
                di * x[2] + dj * y[2],
            ];
            let body = kernel
                .translate(&original.body, offset)
                .map_err(|e| e.to_string())?;
            let names: Names = original
                .names
                .iter()
                .map(|set| {
                    set.iter()
                        .map(|n| FaceName {
                            feature: id,
                            origin: FaceOrigin::Copy {
                                at: (i, j),
                                original: n.to_string(),
                            },
                        })
                        .collect()
                })
                .collect();
            tool = Some(match tool {
                None => (body, names),
                Some((acc, before)) => {
                    combine(kernel, &acc, &before, &body, &names, BooleanOp::Fuse)?
                }
            });
        }
    }
    let (body, names) = tool.ok_or_else(|| String::from("a pattern needs more than one copy"))?;
    Ok((body, names, original.operation))
}

/// Reads an imported solid, naming each face by its number.
fn import_tool(
    kernel: &dyn Kernel,
    id: FeatureId,
    data: &ImportData,
    format: ImportFormat,
) -> Result<(Body, Names), String> {
    let format = match format {
        ImportFormat::Step => cndt_kernel::SolidFormat::Step,
        ImportFormat::Iges => cndt_kernel::SolidFormat::Iges,
    };
    let body = kernel
        .read_solid(&data.0, format)
        .map_err(|error| format!("the imported file cannot be read: {error}"))?;
    let faces = kernel
        .properties(&body)
        .map_err(|error| error.to_string())?
        .faces;
    let names = (0..faces)
        .map(|face| {
            BTreeSet::from([FaceName {
                feature: id,
                origin: FaceOrigin::Imported { face },
            }])
        })
        .collect();
    Ok((body, names))
}

/// Joins a tool to the part or cuts it away, carrying names.
fn apply_tool(
    kernel: &dyn Kernel,
    part: Option<&(Body, Names)>,
    tool: &Body,
    names: &Names,
    operation: Operation,
) -> Result<(Body, Names), String> {
    match (part, operation) {
        (None, Operation::Join) => Ok((tool.clone(), names.clone())),
        (None, Operation::Cut) => Err(String::from(
            "there is no body to cut yet; add a joining extrude first",
        )),
        (Some((body, before)), Operation::Join) => {
            combine(kernel, body, before, tool, names, BooleanOp::Fuse)
        }
        (Some((body, before)), Operation::Cut) => {
            combine(kernel, body, before, tool, names, BooleanOp::Cut)
        }
    }
}

/// Rebuilds the part from the timeline.
#[must_use]
pub fn rebuild(document: &Document, kernel: &dyn Kernel) -> Rebuilt {
    rebuild_to(document, kernel, None)
}

/// Rebuilds the part from the first `count` features of the timeline only,
/// or all of them for `None`: the part as it was at that point of its
/// history. Later features get no status.
#[must_use]
pub fn rebuild_to(document: &Document, kernel: &dyn Kernel, count: Option<usize>) -> Rebuilt {
    let mut part: Option<(Body, Names)> = None;
    let mut status = BTreeMap::new();
    let mut frames = BTreeMap::new();
    let mut tools: BTreeMap<FeatureId, Tool> = BTreeMap::new();
    let mut references = BTreeMap::new();
    let features = document.features();
    let features = &features[..count.map_or(features.len(), |n| n.min(features.len()))];
    for entry in features {
        let outcome = (|| -> Result<Option<(Body, Names)>, String> {
            let Some(sketch) = entry.feature.sketch() else {
                if let Feature::Reference { data, .. } = &entry.feature {
                    let mesh = reference_mesh(data)
                        .map_err(|reason| format!("the mesh cannot be read: {reason}"))?;
                    references.insert(entry.id, mesh);
                    return Ok(part.clone());
                }
                if let Feature::Import {
                    data,
                    operation,
                    format,
                    ..
                } = &entry.feature
                {
                    let (tool, names) = import_tool(kernel, entry.id, data, *format)?;
                    return apply_tool(kernel, part.as_ref(), &tool, &names, *operation).map(Some);
                }
                if let Feature::Pattern { .. } = &entry.feature {
                    let (tool, names, operation) =
                        pattern_tool(kernel, entry.id, &entry.feature, &tools)?;
                    return apply_tool(kernel, part.as_ref(), &tool, &names, operation).map(Some);
                }
                // Chamfers and fillets work on the part itself.
                let (body, names) = part.as_ref().ok_or_else(|| {
                    String::from("there is no body to blend yet; add an extrude first")
                })?;
                return blend_feature(kernel, entry.id, &entry.feature, body, names).map(Some);
            };
            let record = document
                .sketch(sketch)
                .ok_or_else(|| format!("its sketch ({sketch}) has been deleted"))?;
            let placed = resolve(&record.plane, part.as_ref(), kernel);
            frames.entry(sketch).or_insert_with(|| match &placed {
                Ok(frame) => FrameStatus::Ok { frame: *frame },
                Err(reason) => FrameStatus::Failed {
                    reason: reason.clone(),
                },
            });
            let frame =
                placed.map_err(|reason| format!("{} cannot be placed: {reason}", record.name))?;
            let (tool, names, operation) = match &entry.feature {
                Feature::Extrude { operation, .. } => {
                    let (tool, names) =
                        extrude_tool(document, kernel, entry.id, &entry.feature, &frame)?;
                    (tool, names, *operation)
                }
                Feature::Revolve { operation, .. } => {
                    let (tool, names) =
                        revolve_tool(document, kernel, entry.id, &entry.feature, &frame)?;
                    (tool, names, *operation)
                }
                Feature::Hole { .. } => {
                    let (body, _) = part.as_ref().ok_or_else(|| {
                        String::from("there is no body to drill yet; add an extrude first")
                    })?;
                    let (tool, names) =
                        hole_tool(document, kernel, entry.id, &entry.feature, &frame, body)?;
                    (tool, names, Operation::Cut)
                }
                Feature::Chamfer { .. }
                | Feature::Fillet { .. }
                | Feature::Pattern { .. }
                | Feature::Import { .. }
                | Feature::Reference { .. } => {
                    return Err(String::from("this feature has no sketch"));
                }
            };
            tools.insert(
                entry.id,
                Tool {
                    body: tool.clone(),
                    names: names.clone(),
                    operation,
                    frame,
                },
            );
            apply_tool(kernel, part.as_ref(), &tool, &names, operation).map(Some)
        })();
        match outcome {
            Ok(next) => {
                part = next;
                status.insert(entry.id, FeatureStatus::Ok);
            }
            Err(reason) => {
                status.insert(entry.id, FeatureStatus::Failed { reason });
            }
        }
    }
    // Sketches no feature uses yet sit on the finished part.
    for (id, record) in document.sketches() {
        frames
            .entry(*id)
            .or_insert_with(|| match resolve(&record.plane, part.as_ref(), kernel) {
                Ok(frame) => FrameStatus::Ok { frame },
                Err(reason) => FrameStatus::Failed { reason },
            });
    }
    let properties = part
        .as_ref()
        .and_then(|(body, _)| kernel.properties(body).ok());
    let (body, faces) = match part {
        Some((body, names)) => (Some(body), names),
        None => (None, Vec::new()),
    };
    Rebuilt {
        body,
        faces,
        status,
        properties,
        frames,
        references,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;
    use cndt_kernel::OpenCascade;
    use cndt_sketch::{Constraint, Point2, SolveOptions};

    fn rectangle(s: &mut Sketch, x: f64, y: f64, w: f64, h: f64) -> Vec<EntityId> {
        let p: Vec<EntityId> = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
            .iter()
            .map(|&(px, py)| s.add_point(Point2::new(px, py)).unwrap())
            .collect();
        (0..4)
            .map(|i| s.add_line(p[i], p[(i + 1) % 4]).unwrap())
            .collect()
    }

    fn extrude(
        name: &str,
        sketch: SketchId,
        profiles: Vec<ProfileKey>,
        distance: f64,
        operation: Operation,
    ) -> Feature {
        Feature::Extrude {
            name: name.into(),
            sketch,
            profiles,
            distance,
            distance_expression: None,
            operation,
        }
    }

    /// The test block's body, pocket and a hole (decision 0010).
    fn block() -> (Document, SketchId, Vec<EntityId>) {
        let mut d = Document::new("Test block");
        let base = d.add_sketch("", Plane::Xy).unwrap();
        let outline = d
            .edit_sketch(base, |s| Ok(rectangle(s, 0.0, 0.0, 96.0, 56.0)))
            .unwrap();
        let profiles = default_profiles(&d.sketch(base).unwrap().sketch);
        d.add_feature(extrude("Extrude1", base, profiles, 20.0, Operation::Join))
            .unwrap();
        (d, base, outline)
    }

    #[test]
    fn a_block_with_named_faces() {
        let (d, _, outline) = block();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(r.status.get(&FeatureId(0)), Some(&FeatureStatus::Ok));
        let p = r.properties.unwrap();
        assert!((p.volume - 96.0 * 56.0 * 20.0).abs() < 1e-6);
        assert_eq!(r.faces.len(), 6);
        let tokens: BTreeSet<String> = r.faces.iter().flatten().map(ToString::to_string).collect();
        for line in &outline {
            assert!(
                tokens.contains(&format!("f0/side/e{}", line.0)),
                "{tokens:?}"
            );
        }
        assert!(tokens.iter().any(|t| t.starts_with("f0/start/")));
        assert!(tokens.iter().any(|t| t.starts_with("f0/end/")));
    }

    #[test]
    fn a_pocket_cut_keeps_the_block_names_and_adds_its_own() {
        let (mut d, _, outline) = block();
        let top = d.add_sketch("Pocket", Plane::Xy).unwrap();
        d.edit_sketch(top, |s| Ok(rectangle(s, 28.0, 18.0, 40.0, 20.0)))
            .unwrap();
        let profiles = default_profiles(&d.sketch(top).unwrap().sketch);
        // The pocket sketch is on Z = 0 for now (sketches on faces come
        // later), so the cut goes up into the block from below.
        d.add_feature(extrude("Pocket", top, profiles, 8.0, Operation::Cut))
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(1)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let v = r.properties.unwrap().volume;
        assert!(
            (v - (96.0 * 56.0 * 20.0 - 40.0 * 20.0 * 8.0)).abs() < 1e-6,
            "{v}"
        );
        let tokens: BTreeSet<String> = r.faces.iter().flatten().map(ToString::to_string).collect();
        for line in &outline {
            assert!(tokens.contains(&format!("f0/side/e{}", line.0)));
        }
        assert_eq!(
            tokens.iter().filter(|t| t.starts_with("f1/side/")).count(),
            4,
            "pocket walls"
        );
        assert!(
            tokens.iter().any(|t| t.starts_with("f1/end/")),
            "pocket floor"
        );
        // Every face is named, and each name is on exactly one face.
        assert!(r.faces.iter().all(|n| !n.is_empty()));
        for t in &tokens {
            let on = r
                .faces
                .iter()
                .filter(|n| n.iter().any(|x| &x.to_string() == t))
                .count();
            assert_eq!(on, 1, "{t}");
        }
    }

    /// The test block's pocket, drawn on the block's top face and cut down
    /// into it (decision 0010, feature 3).
    #[test]
    fn a_pocket_sketched_on_the_top_face() {
        let (mut d, _, _) = block();
        let r = rebuild(&d, &OpenCascade);
        let top = r
            .faces
            .iter()
            .flatten()
            .map(ToString::to_string)
            .find(|t| t.starts_with("f0/end/"))
            .unwrap();
        let s = d.add_sketch("Pocket", Plane::Face(top.clone())).unwrap();
        d.edit_sketch(s, |k| Ok(rectangle(k, 28.0, 18.0, 40.0, 20.0)))
            .unwrap();
        let profiles = default_profiles(&d.sketch(s).unwrap().sketch);
        d.add_feature(extrude("Pocket", s, profiles, -8.0, Operation::Cut))
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(1)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let unused = d.add_sketch("Later", Plane::Face(top.clone())).unwrap();
        let later = rebuild(&d, &OpenCascade);
        assert!(
            matches!(later.frames.get(&unused), Some(FrameStatus::Ok { .. })),
            "a sketch no feature uses yet is placed on the finished part"
        );
        let Some(FrameStatus::Ok { frame }) = r.frames.get(&s) else {
            panic!("{:?}", r.frames)
        };
        assert_eq!(
            frame.at(28.0, 18.0),
            Point3::new(28.0, 18.0, 20.0),
            "top-face coordinates are model X and Y"
        );
        let p = r.properties.unwrap();
        assert!((p.volume - (96.0 * 56.0 * 20.0 - 40.0 * 20.0 * 8.0)).abs() < 1e-6);
        assert!(
            (p.bounds.max.z - 20.0).abs() < 1e-9,
            "cut down from the top, not up from below"
        );
        // The top face, now with the pocket opening in it, keeps its name,
        // so the sketch still finds it after an edit to the block.
        assert!(r.faces.iter().flatten().any(|n| n.to_string() == top));
        let mut taller = d.features()[0].feature.clone();
        let Feature::Extrude { distance: dd, .. } = &mut taller else {
            panic!()
        };
        *dd += 5.0;
        d.replace_feature(FeatureId(0), taller).unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(r.status.get(&FeatureId(1)), Some(&FeatureStatus::Ok));
        assert!((r.properties.unwrap().bounds.max.z - 25.0).abs() < 1e-9);
    }

    fn top_face(d: &Document) -> String {
        rebuild(d, &OpenCascade)
            .faces
            .iter()
            .flatten()
            .map(ToString::to_string)
            .find(|t| t.starts_with("f0/end/"))
            .unwrap()
    }

    /// Points on a new sketch on the block's top face.
    fn top_points(d: &mut Document, at: &[(f64, f64)]) -> (SketchId, Vec<EntityId>) {
        let face = top_face(d);
        let s = d.add_sketch("", Plane::Face(face)).unwrap();
        let points = d
            .edit_sketch(s, |k| {
                at.iter()
                    .map(|&(x, y)| k.add_point(Point2::new(x, y)))
                    .collect()
            })
            .unwrap();
        (s, points)
    }

    fn hole(name: &str, sketch: SketchId, points: Vec<EntityId>, spec: HoleSpec) -> Feature {
        Feature::Hole {
            name: name.into(),
            sketch,
            points,
            spec,
        }
    }

    fn simple(diameter: f64, depth: HoleDepth) -> HoleSpec {
        HoleSpec {
            diameter,
            depth,
            tip_angle: 118.0,
            counterbore: None,
            thread: None,
            fit: None,
        }
    }

    /// The test block's holes (decision 0010, features 7 to 10).
    #[test]
    fn the_test_block_holes() {
        let pi = std::f64::consts::PI;
        let (mut d, _, _) = block();
        let block_volume = 96.0 * 56.0 * 20.0;
        // 4 × Ø6.6 clearance, through.
        let (s1, p1) = top_points(
            &mut d,
            &[(8.0, 8.0), (88.0, 8.0), (8.0, 48.0), (88.0, 48.0)],
        );
        d.add_feature(hole(
            "Clearance",
            s1,
            p1.clone(),
            simple(6.6, HoleDepth::Through),
        ))
        .unwrap();
        // 4 × M6 × 1.0 tapped, 12 mm thread in a 15 mm tap-drill hole.
        let (s2, p2) = top_points(
            &mut d,
            &[(20.0, 8.0), (76.0, 8.0), (20.0, 48.0), (76.0, 48.0)],
        );
        let tapped = HoleSpec {
            thread: Some(Thread {
                size: "M6x1.0".into(),
                depth: 12.0,
            }),
            ..simple(
                crate::threads::thread("M6").unwrap().tap_drill(),
                HoleDepth::Blind(15.0),
            )
        };
        d.add_feature(hole("Tapped", s2, p2.clone(), tapped))
            .unwrap();
        // 2 × counterbored for M5 cap screws.
        let (s3, p3) = top_points(&mut d, &[(30.0, 28.0), (66.0, 28.0)]);
        let counterbored = HoleSpec {
            counterbore: Some(Counterbore {
                diameter: 9.5,
                depth: 5.4,
            }),
            ..simple(5.5, HoleDepth::Through)
        };
        d.add_feature(hole("Counterbored", s3, p3.clone(), counterbored))
            .unwrap();
        // 1 × Ø8 H7 dowel, 12 deep with a flat-bottomed ream.
        let (s4, p4) = top_points(&mut d, &[(48.0, 40.0)]);
        let dowel = HoleSpec {
            fit: Some("H7".into()),
            tip_angle: 180.0,
            ..simple(8.0, HoleDepth::Blind(12.0))
        };
        d.add_feature(hole("Dowel", s4, p4.clone(), dowel)).unwrap();

        let r = rebuild(&d, &OpenCascade);
        assert!(
            r.status.values().all(|s| *s == FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let tip = |dia: f64, angle: f64| {
            pi * (dia / 2.0).powi(2) * (dia / 2.0 / (angle / 2.0f64).to_radians().tan()) / 3.0
        };
        let expected = block_volume
            - 4.0 * pi * 3.3 * 3.3 * 20.0
            - 4.0 * (pi * 2.5 * 2.5 * 15.0 + tip(5.0, 118.0))
            - 2.0 * (pi * 4.75 * 4.75 * 5.4 + pi * 2.75 * 2.75 * (20.0 - 5.4))
            - pi * 16.0 * 12.0;
        let v = r.properties.unwrap().volume;
        assert!((v - expected).abs() < 1e-3, "{v} vs {expected}");
        // Every hole surface is named, once.
        let tokens: Vec<String> = r.faces.iter().flatten().map(ToString::to_string).collect();
        for p in &p1 {
            assert!(
                tokens.contains(&format!("f1/hole/e{}/wall", p.0)),
                "{tokens:?}"
            );
        }
        for p in &p2 {
            assert!(tokens.contains(&format!("f2/hole/e{}/tip", p.0)));
        }
        for p in &p3 {
            assert!(tokens.contains(&format!("f3/hole/e{}/counterbore_wall", p.0)));
            assert!(tokens.contains(&format!("f3/hole/e{}/counterbore_floor", p.0)));
        }
        assert!(tokens.contains(&format!("f4/hole/e{}/floor", p4[0].0)));
        let mut sorted = tokens.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), tokens.len(), "no name on two faces");
    }

    /// 0.5 mm chamfers on the top outside edges and every hole (decision
    /// 0010, feature 11), and a filleted corner.
    #[test]
    fn chamfer_the_top_face_and_fillet_a_corner() {
        let (mut d, _, outline) = block();
        let (s, p) = top_points(&mut d, &[(20.0, 20.0), (70.0, 30.0)]);
        d.add_feature(hole("Holes", s, p.clone(), simple(6.6, HoleDepth::Through)))
            .unwrap();
        // Round the corner first, then chamfer, as a machinist would order them.
        let side = |e: EntityId| format!("f0/side/e{}", e.0);
        d.add_feature(Feature::Fillet {
            name: "Fillet1".into(),
            edges: vec![EdgeTarget::Between(side(outline[0]), side(outline[1]))],
            radius: 3.0,
        })
        .unwrap();
        let top = top_face(&d);
        d.add_feature(Feature::Chamfer {
            name: "Chamfer1".into(),
            edges: vec![EdgeTarget::Face(top.clone())],
            distance: 0.5,
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert!(
            r.status.values().all(|s| *s == FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let tokens: Vec<String> = r.faces.iter().flatten().map(ToString::to_string).collect();
        assert!(
            tokens.iter().any(|t| t.starts_with("f2/blend/")),
            "the fillet face"
        );
        let chamfers = tokens.iter().filter(|t| t.starts_with("f3/blend/")).count();
        assert_eq!(
            chamfers, 7,
            "4 outside edges, the rounded corner and 2 hole rims: {tokens:?}"
        );
        // 6 block + 2 hole walls + 1 fillet + 7 chamfers.
        assert_eq!(r.faces.len(), 16);
        let mut sorted = tokens.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), tokens.len(), "no name on two faces");

        // An edge that no longer exists is flagged, not guessed.
        d.add_feature(Feature::Fillet {
            name: "Ghost".into(),
            edges: vec![EdgeTarget::Between(side(outline[0]), side(outline[2]))],
            radius: 1.0,
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        let Some(FeatureStatus::Failed { reason }) = r.status.get(&FeatureId(4)) else {
            panic!("{:?}", r.status)
        };
        assert!(reason.contains("no longer meet"), "{reason}");
    }

    /// A row of holes made by patterning one, and a grid of bosses.
    #[test]
    fn patterns_repeat_holes_and_bosses() {
        let pi = std::f64::consts::PI;
        let (mut d, _, _) = block();
        let (s, p) = top_points(&mut d, &[(10.0, 10.0)]);
        d.add_feature(hole("Hole", s, p.clone(), simple(5.0, HoleDepth::Through)))
            .unwrap();
        d.add_feature(Feature::Pattern {
            name: "Row".into(),
            feature: FeatureId(1),
            count_x: 4,
            spacing_x: 20.0,
            count_y: 2,
            spacing_y: 30.0,
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert!(
            r.status.values().all(|s| *s == FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let v = r.properties.unwrap().volume;
        let expected = 96.0 * 56.0 * 20.0 - 8.0 * pi * 2.5 * 2.5 * 20.0;
        assert!((v - expected).abs() < 1e-3, "{v} vs {expected}");
        let tokens: Vec<String> = r.faces.iter().flatten().map(ToString::to_string).collect();
        let wall = format!("f1/hole/e{}/wall", p[0].0);
        assert!(tokens.contains(&wall));
        assert!(tokens.contains(&format!("f2/at3,1/{wall}")), "{tokens:?}");
        assert_eq!(
            tokens.iter().filter(|t| t.starts_with("f2/at")).count(),
            7,
            "7 copies, one wall each"
        );

        // A pattern of something that failed is flagged too.
        d.add_feature(Feature::Pattern {
            name: "Of nothing".into(),
            feature: FeatureId(9),
            count_x: 2,
            spacing_x: 5.0,
            count_y: 1,
            spacing_y: 0.0,
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert!(matches!(
            r.status.get(&FeatureId(3)),
            Some(FeatureStatus::Failed { .. })
        ));
        // Bad grids are refused.
        let bad = Feature::Pattern {
            name: "Bad".into(),
            feature: FeatureId(1),
            count_x: 3,
            spacing_x: 0.0,
            count_y: 1,
            spacing_y: 0.0,
        };
        assert!(
            d.add_feature(bad)
                .unwrap_err()
                .to_string()
                .contains("spacing")
        );
    }

    #[test]
    fn bad_holes_are_refused_with_a_reason() {
        let (mut d, _, _) = block();
        let (s, p) = top_points(&mut d, &[(10.0, 10.0)]);
        let cases = [
            (
                HoleSpec {
                    counterbore: Some(Counterbore {
                        diameter: 4.0,
                        depth: 2.0,
                    }),
                    ..simple(5.5, HoleDepth::Through)
                },
                "wider",
            ),
            (
                HoleSpec {
                    thread: Some(Thread {
                        size: "M7".into(),
                        depth: 5.0,
                    }),
                    ..simple(5.0, HoleDepth::Blind(10.0))
                },
                "unknown thread",
            ),
            (
                HoleSpec {
                    thread: Some(Thread {
                        size: "M6".into(),
                        depth: 12.0,
                    }),
                    ..simple(5.0, HoleDepth::Blind(10.0))
                },
                "deeper than the hole",
            ),
            (
                HoleSpec {
                    thread: Some(Thread {
                        size: "M6".into(),
                        depth: 5.0,
                    }),
                    ..simple(6.0, HoleDepth::Blind(10.0))
                },
                "smaller than 6",
            ),
            (simple(-1.0, HoleDepth::Through), "more than zero"),
            (
                HoleSpec {
                    tip_angle: 0.0,
                    ..simple(5.0, HoleDepth::Through)
                },
                "point angle",
            ),
        ];
        for (spec, expected) in cases {
            let error = d.add_feature(hole("Bad", s, p.clone(), spec)).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
        // A hole before any body has nothing to drill.
        let mut empty = Document::default();
        let s = empty.add_sketch("", Plane::Xy).unwrap();
        let p = empty
            .edit_sketch(s, |k| k.add_point(Point2::new(0.0, 0.0)))
            .unwrap();
        empty
            .add_feature(hole("Early", s, vec![p], simple(5.0, HoleDepth::Through)))
            .unwrap();
        let r = rebuild(&empty, &OpenCascade);
        assert!(
            matches!(r.status.get(&FeatureId(0)), Some(FeatureStatus::Failed { reason }) if reason.contains("nothing") || reason.contains("no body"))
        );
    }

    #[test]
    fn a_sketch_on_a_missing_face_is_flagged() {
        let (mut d, _, _) = block();
        let s = d
            .add_sketch("Ghost", Plane::Face(String::from("f9/end/1-2-3-4")))
            .unwrap();
        d.edit_sketch(s, |k| Ok(rectangle(k, 0.0, 0.0, 5.0, 5.0)))
            .unwrap();
        let profiles = default_profiles(&d.sketch(s).unwrap().sketch);
        d.add_feature(extrude("Boss", s, profiles, 5.0, Operation::Join))
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        let Some(FeatureStatus::Failed { reason }) = r.status.get(&FeatureId(1)) else {
            panic!()
        };
        assert!(reason.contains("no longer exists"), "{reason}");
        assert!(matches!(r.frames.get(&s), Some(FrameStatus::Failed { .. })));
    }

    #[test]
    fn names_survive_a_dimension_change() {
        let (mut d, base, outline) = block();
        let before: Vec<BTreeSet<String>> = rebuild(&d, &OpenCascade)
            .faces
            .iter()
            .map(|n| n.iter().map(ToString::to_string).collect())
            .collect();
        let opts = SolveOptions::default();
        d.edit_sketch(base, |s| {
            let c = s.add_constraint(
                Constraint::Length {
                    line: outline[0],
                    value: 96.0,
                },
                &opts,
            )?;
            s.set_dimension(c, 120.0, &opts)
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        let after: Vec<BTreeSet<String>> = r
            .faces
            .iter()
            .map(|n| n.iter().map(ToString::to_string).collect())
            .collect();
        let mut a = before.clone();
        let mut b = after.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "the same names, on a bigger block");
        assert!((r.properties.unwrap().bounds.size()[0] - 120.0).abs() < 1e-6);
    }

    #[test]
    fn a_plate_with_a_hole_by_default() {
        let mut d = Document::default();
        let s = d.add_sketch("", Plane::Xy).unwrap();
        d.edit_sketch(s, |k| {
            rectangle(k, 0.0, 0.0, 40.0, 30.0);
            let c = k.add_point(Point2::new(20.0, 15.0))?;
            k.add_circle(c, 3.3)
        })
        .unwrap();
        let profiles = default_profiles(&d.sketch(s).unwrap().sketch);
        assert_eq!(profiles.len(), 1, "the plate, not the disc in the hole");
        d.add_feature(extrude("Plate", s, profiles, 5.0, Operation::Join))
            .unwrap();
        let v = rebuild(&d, &OpenCascade).properties.unwrap().volume;
        let expected = 40.0 * 30.0 * 5.0 - std::f64::consts::PI * 3.3 * 3.3 * 5.0;
        assert!((v - expected).abs() < 1e-3, "{v} vs {expected}");
    }

    #[test]
    fn broken_features_are_flagged_and_the_rest_still_build() {
        let (mut d, base, outline) = block();
        // A cut first in a fresh document has nothing to cut.
        let mut empty = Document::default();
        let s = empty.add_sketch("", Plane::Xy).unwrap();
        empty
            .edit_sketch(s, |k| Ok(rectangle(k, 0.0, 0.0, 5.0, 5.0)))
            .unwrap();
        let p = default_profiles(&empty.sketch(s).unwrap().sketch);
        empty
            .add_feature(extrude("Cut", s, p, 1.0, Operation::Cut))
            .unwrap();
        let r = rebuild(&empty, &OpenCascade);
        assert!(
            matches!(r.status.get(&FeatureId(0)), Some(FeatureStatus::Failed { reason }) if reason.contains("no body"))
        );
        assert!(r.body.is_none());

        // Opening the outline breaks the block's profile: flagged, not guessed.
        let second = d.add_sketch("", Plane::Xz).unwrap();
        d.edit_sketch(second, |k| Ok(rectangle(k, 0.0, 0.0, 10.0, 10.0)))
            .unwrap();
        let p = default_profiles(&d.sketch(second).unwrap().sketch);
        d.add_feature(extrude("Fin", second, p, -5.0, Operation::Join))
            .unwrap();
        d.edit_sketch(base, |k| k.remove_entity(outline[0]).map(|_| ()))
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        let Some(FeatureStatus::Failed { reason }) = r.status.get(&FeatureId(0)) else {
            panic!("{:?}", r.status)
        };
        assert!(reason.contains("no longer closed"), "{reason}");
        assert_eq!(
            r.status.get(&FeatureId(1)),
            Some(&FeatureStatus::Ok),
            "the fin still builds"
        );
        let size = r.properties.unwrap().bounds.size();
        assert!(
            (size[1] - 5.0).abs() < 1e-6,
            "the fin goes +Y from the XZ plane: {size:?}"
        );
    }

    fn import(name: &str, content: Vec<u8>, operation: Operation) -> Feature {
        let data = ImportData(std::sync::Arc::new(content));
        let (bytes, hash) = data.fingerprint();
        Feature::Import {
            name: name.into(),
            source: String::from("box-10x20x30.step"),
            format: ImportFormat::Step,
            operation,
            bytes,
            hash,
            data,
        }
    }

    fn box_step() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/box-10x20x30.step"),
        )
        .unwrap()
    }

    #[test]
    fn an_imported_solid_names_its_faces_and_takes_later_features() {
        let mut d = Document::new("Imported");
        d.add_feature(import("Import1", box_step(), Operation::Join))
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(r.status.get(&FeatureId(0)), Some(&FeatureStatus::Ok));
        let tokens: Vec<String> = r.faces.iter().flatten().map(ToString::to_string).collect();
        assert_eq!(
            tokens,
            (0..6).map(|n| format!("f0/import/{n}")).collect::<Vec<_>>()
        );
        let volume = r.properties.unwrap().volume;
        assert!((volume - 6000.0).abs() < 1e-6, "{volume}");
        // A chamfer on an imported face's edges builds, and names its faces.
        d.add_feature(Feature::Chamfer {
            name: "Chamfer1".into(),
            edges: vec![EdgeTarget::Face(String::from("f0/import/0"))],
            distance: 1.0,
        })
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert!(
            r.status.values().all(|s| *s == FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        assert!(r.properties.unwrap().volume < 6000.0);
        assert_eq!(r.faces.len(), 10, "6 faces and 4 chamfers");
    }

    #[test]
    fn an_unreadable_import_is_flagged_and_the_rest_still_build() {
        let mut d = Document::new("Imported");
        d.add_feature(import("Import1", box_step(), Operation::Join))
            .unwrap();
        d.add_feature(import(
            "Import2",
            b"ISO-10303-21; nothing".to_vec(),
            Operation::Cut,
        ))
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(r.status.get(&FeatureId(0)), Some(&FeatureStatus::Ok));
        match r.status.get(&FeatureId(1)) {
            Some(FeatureStatus::Failed { reason }) => {
                assert!(reason.contains("cannot be read"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        assert!((r.properties.unwrap().volume - 6000.0).abs() < 1e-6);
    }

    #[test]
    fn a_reference_mesh_never_changes_the_part() {
        let stl = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 40 0 0\nvertex 0 30 5\nendloop\nendfacet\nendsolid t\n";
        let reference = |name: &str, content: &[u8]| {
            let data = ImportData(std::sync::Arc::new(content.to_vec()));
            let (bytes, hash) = data.fingerprint();
            Feature::Reference {
                name: name.into(),
                source: String::from("scan.stl"),
                bytes,
                hash,
                data,
            }
        };
        let mut d = Document::new("Mesh");
        d.add_feature(reference("Mesh1", stl.as_bytes())).unwrap();
        d.add_feature(import("Import1", box_step(), Operation::Join))
            .unwrap();
        d.add_feature(reference("Mesh2", b"not a mesh")).unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(r.status.get(&FeatureId(0)), Some(&FeatureStatus::Ok));
        assert_eq!(r.status.get(&FeatureId(1)), Some(&FeatureStatus::Ok));
        assert!(matches!(
            r.status.get(&FeatureId(2)),
            Some(FeatureStatus::Failed { reason }) if reason.contains("cannot be read")
        ));
        assert!((r.properties.unwrap().volume - 6000.0).abs() < 1e-6);
        assert_eq!(r.faces.len(), 6, "the mesh adds no faces");
        let mesh = &r.references[&FeatureId(0)];
        assert_eq!(mesh.mesh.triangle_count(), 1);
        assert_eq!(mesh.bounds.size(), [40.0, 30.0, 5.0]);
        assert_eq!(r.references.len(), 1);
        // Rolled back to before it, the mesh is not there.
        assert!(rebuild_to(&d, &OpenCascade, Some(0)).references.is_empty());
    }

    /// A polygon through the given points (x, y), closed, in `s`.
    fn polygon(s: &mut Sketch, points: &[(f64, f64)]) -> Vec<EntityId> {
        let p: Vec<EntityId> = points
            .iter()
            .map(|&(x, y)| s.add_point(Point2::new(x, y)).unwrap())
            .collect();
        (0..p.len())
            .map(|i| s.add_line(p[i], p[(i + 1) % p.len()]).unwrap())
            .collect()
    }

    fn revolve(
        sketch: SketchId,
        d: &Document,
        axis: RevolveAxis,
        angle: f64,
        operation: Operation,
    ) -> Feature {
        Feature::Revolve {
            name: "Revolve1".into(),
            sketch,
            profiles: default_profiles(&d.sketch(sketch).unwrap().sketch),
            axis,
            angle,
            angle_expression: None,
            operation,
        }
    }

    fn tokens(r: &Rebuilt) -> BTreeSet<String> {
        r.faces.iter().flatten().map(ToString::to_string).collect()
    }

    #[test]
    fn a_stepped_shaft_by_revolving_its_half_section() {
        let mut d = Document::new("Shaft");
        let sk = d.add_sketch("Section", Plane::Xy).unwrap();
        let lines = d
            .edit_sketch(sk, |s| {
                Ok(polygon(
                    s,
                    &[
                        (0.0, 0.0),
                        (10.0, 0.0),
                        (10.0, 20.0),
                        (6.0, 20.0),
                        (6.0, 30.0),
                        (0.0, 30.0),
                    ],
                ))
            })
            .unwrap();
        let f = revolve(sk, &d, RevolveAxis::SketchY, 360.0, Operation::Join);
        d.add_feature(f).unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(0)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let volume = std::f64::consts::PI * (100.0 * 20.0 + 36.0 * 10.0);
        assert!((r.properties.unwrap().volume - volume).abs() < 1e-6);
        // Two ends, two walls and the shoulder: each named by the sketch
        // line that swept it. The line on the axis swept nothing.
        assert_eq!(r.faces.len(), 5);
        let names = tokens(&r);
        for line in &lines[..4] {
            assert!(names.contains(&format!("f0/side/e{}", line.0)), "{names:?}");
        }
        assert!(
            !names
                .iter()
                .any(|n| n.contains("/start/") || n.contains("/end/"))
        );
    }

    #[test]
    fn a_revolved_arc_is_exact_and_a_line_of_the_sketch_can_be_the_axis() {
        // A half disc about its diameter: a sphere, one face.
        let mut d = Document::new("Ball");
        let sk = d.add_sketch("Half", Plane::Xy).unwrap();
        d.edit_sketch(sk, |s| {
            let c = s.add_point(Point2::new(0.0, 0.0))?;
            let a = s.add_point(Point2::new(0.0, -5.0))?;
            let b = s.add_point(Point2::new(0.0, 5.0))?;
            s.add_arc(c, a, b)?;
            s.add_line(b, a)?;
            Ok(())
        })
        .unwrap();
        d.add_feature(revolve(
            sk,
            &d,
            RevolveAxis::SketchY,
            360.0,
            Operation::Join,
        ))
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(0)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let sphere = 4.0 / 3.0 * std::f64::consts::PI * 125.0;
        assert!((r.properties.unwrap().volume - sphere).abs() < 1e-6);
        assert_eq!(r.faces.len(), 1);

        // A tube: a rectangle beside a line of the same sketch, which turns it.
        let mut d = Document::new("Tube");
        let sk = d.add_sketch("Wall", Plane::Xy).unwrap();
        let axis = d
            .edit_sketch(sk, |s| {
                rectangle(s, 5.0, 0.0, 5.0, 12.0);
                let a = s.add_point(Point2::new(0.0, -3.0))?;
                let b = s.add_point(Point2::new(0.0, 15.0))?;
                s.add_line(a, b)
            })
            .unwrap();
        d.add_feature(revolve(
            sk,
            &d,
            RevolveAxis::Line { entity: axis },
            360.0,
            Operation::Join,
        ))
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(0)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let tube = std::f64::consts::PI * (100.0 - 25.0) * 12.0;
        assert!((r.properties.unwrap().volume - tube).abs() < 1e-6);
        assert_eq!(r.faces.len(), 4);
    }

    #[test]
    fn a_partial_turn_has_caps_and_a_cut_revolve_removes_material() {
        let mut d = Document::new("Quarter");
        let sk = d.add_sketch("Section", Plane::Xy).unwrap();
        d.edit_sketch(sk, |s| Ok(rectangle(s, 0.0, 0.0, 5.0, 10.0)))
            .unwrap();
        d.add_feature(revolve(sk, &d, RevolveAxis::SketchY, 90.0, Operation::Join))
            .unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(0)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let quarter = std::f64::consts::PI * 25.0 * 10.0 / 4.0;
        assert!((r.properties.unwrap().volume - quarter).abs() < 1e-6);
        let names = tokens(&r);
        assert!(
            names.iter().any(|n| n.starts_with("f0/start/")),
            "{names:?}"
        );
        assert!(names.iter().any(|n| n.starts_with("f0/end/")), "{names:?}");

        // A full cylinder with a ring cut out of it by a revolve.
        let mut d = Document::new("Groove");
        let body = d.add_sketch("Body", Plane::Xy).unwrap();
        d.edit_sketch(body, |s| Ok(rectangle(s, 0.0, 0.0, 10.0, 20.0)))
            .unwrap();
        d.add_feature(revolve(
            body,
            &d,
            RevolveAxis::SketchY,
            360.0,
            Operation::Join,
        ))
        .unwrap();
        let groove = d.add_sketch("Groove", Plane::Xy).unwrap();
        d.edit_sketch(groove, |s| Ok(rectangle(s, 8.0, 8.0, 3.0, 4.0)))
            .unwrap();
        let mut cut = revolve(groove, &d, RevolveAxis::SketchY, 360.0, Operation::Cut);
        if let Feature::Revolve { name, .. } = &mut cut {
            *name = "Groove".into();
        }
        d.add_feature(cut).unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(
            r.status.get(&FeatureId(1)),
            Some(&FeatureStatus::Ok),
            "{:?}",
            r.status
        );
        let groove_volume = std::f64::consts::PI * (100.0 - 64.0) * 4.0;
        let cylinder = std::f64::consts::PI * 100.0 * 20.0;
        assert!((r.properties.unwrap().volume - (cylinder - groove_volume)).abs() < 1e-6);
        // The groove's walls carry the cut feature's name; the cylinder's
        // keep the first feature's.
        let names = tokens(&r);
        assert!(names.iter().any(|n| n.starts_with("f1/side/")), "{names:?}");
        assert!(names.iter().any(|n| n.starts_with("f0/side/")), "{names:?}");
    }

    #[test]
    fn failures_say_why() {
        // A profile across the axis would fold into itself.
        let mut d = Document::new("Across");
        let sk = d.add_sketch("Section", Plane::Xy).unwrap();
        d.edit_sketch(sk, |s| Ok(rectangle(s, -2.0, 0.0, 5.0, 10.0)))
            .unwrap();
        d.add_feature(revolve(
            sk,
            &d,
            RevolveAxis::SketchY,
            360.0,
            Operation::Join,
        ))
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        match r.status.get(&FeatureId(0)) {
            Some(FeatureStatus::Failed { reason }) => {
                assert!(reason.contains("crosses the axis"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        // An axis that is not a line of the sketch.
        let mut d = Document::new("Bad axis");
        let sk = d.add_sketch("Section", Plane::Xy).unwrap();
        d.edit_sketch(sk, |s| Ok(rectangle(s, 1.0, 0.0, 5.0, 10.0)))
            .unwrap();
        d.add_feature(revolve(
            sk,
            &d,
            RevolveAxis::Line {
                entity: EntityId(999),
            },
            360.0,
            Operation::Join,
        ))
        .unwrap();
        let r = rebuild(&d, &OpenCascade);
        match r.status.get(&FeatureId(0)) {
            Some(FeatureStatus::Failed { reason }) => {
                assert!(reason.contains("not a line"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        // Angles outside 0 to 360 are refused when the feature is added.
        for angle in [0.0, -5.0, 361.0, f64::NAN] {
            let f = revolve(sk, &d, RevolveAxis::SketchY, angle, Operation::Join);
            assert!(f.check().is_err(), "{angle}");
        }
    }

    #[test]
    fn a_revolve_survives_saving_as_json() {
        let f = Feature::Revolve {
            name: "Revolve1".into(),
            sketch: SketchId(0),
            profiles: Vec::new(),
            axis: RevolveAxis::Line {
                entity: EntityId(4),
            },
            angle: 270.0,
            angle_expression: Some("sweep".into()),
            operation: Operation::Cut,
        };
        let text = serde_json::to_string(&f).unwrap();
        assert!(text.contains("\"type\":\"revolve\""), "{text}");
        assert!(
            text.contains("\"axis\":{\"type\":\"line\",\"entity\":4}"),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Feature>(&text).unwrap(), f);
    }
}
