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

use cndt_geom::Point3;
use cndt_kernel::{Body, BodyProperties, BooleanOp, Curve, Kernel, Made};
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

/// A solid feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Feature {
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
        /// Join or cut.
        operation: Operation,
    },
}

impl Feature {
    /// The timeline name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Extrude { name, .. } => name,
        }
    }

    /// The sketch it uses.
    #[must_use]
    pub const fn sketch(&self) -> SketchId {
        match self {
            Self::Extrude { sketch, .. } => *sketch,
        }
    }

    /// Checks the values the editing commands must keep.
    pub(crate) fn check(&self) -> Result<(), String> {
        match self {
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
    } = feature;
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

/// Rebuilds the part from the timeline.
#[must_use]
pub fn rebuild(document: &Document, kernel: &dyn Kernel) -> Rebuilt {
    let mut part: Option<(Body, Names)> = None;
    let mut status = BTreeMap::new();
    let mut frames = BTreeMap::new();
    for entry in document.features() {
        let outcome = (|| -> Result<Option<(Body, Names)>, String> {
            let Feature::Extrude {
                operation, sketch, ..
            } = &entry.feature;
            let record = document
                .sketch(*sketch)
                .ok_or_else(|| format!("its sketch ({sketch}) has been deleted"))?;
            let placed = resolve(&record.plane, part.as_ref(), kernel);
            frames.entry(*sketch).or_insert_with(|| match &placed {
                Ok(frame) => FrameStatus::Ok { frame: *frame },
                Err(reason) => FrameStatus::Failed {
                    reason: reason.clone(),
                },
            });
            let frame =
                placed.map_err(|reason| format!("{} cannot be placed: {reason}", record.name))?;
            let (tool, names) = extrude_tool(document, kernel, entry.id, &entry.feature, &frame)?;
            Ok(Some(match (&part, operation) {
                (None, Operation::Join) => (tool, names),
                (None, Operation::Cut) => {
                    return Err(String::from(
                        "there is no body to cut yet; add a joining extrude first",
                    ));
                }
                (Some((body, before)), Operation::Join) => {
                    combine(kernel, body, before, &tool, &names, BooleanOp::Fuse)?
                }
                (Some((body, before)), Operation::Cut) => {
                    combine(kernel, body, before, &tool, &names, BooleanOp::Cut)?
                }
            }))
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
        let Feature::Extrude { distance, .. } = d.features()[0].feature.clone();
        let mut taller = d.features()[0].feature.clone();
        let Feature::Extrude { distance: dd, .. } = &mut taller;
        *dd = distance + 5.0;
        d.replace_feature(FeatureId(0), taller).unwrap();
        let r = rebuild(&d, &OpenCascade);
        assert_eq!(r.status.get(&FeatureId(1)), Some(&FeatureStatus::Ok));
        assert!((r.properties.unwrap().bounds.max.z - 25.0).abs() < 1e-9);
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
}
