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

/// Sketch coordinates to model coordinates on an origin plane.
fn to_model(plane: Plane, x: f64, y: f64) -> Point3 {
    match plane {
        Plane::Xy => Point3::new(x, y, 0.0),
        Plane::Xz => Point3::new(x, 0.0, y),
        Plane::Yz => Point3::new(0.0, x, y),
    }
}

/// The plane's normal: sketch X cross sketch Y, so counter-clockwise in the
/// sketch is counter-clockwise about it.
const fn normal(plane: Plane) -> [f64; 3] {
    match plane {
        Plane::Xy => [0.0, 0.0, 1.0],
        Plane::Xz => [0.0, -1.0, 0.0],
        Plane::Yz => [1.0, 0.0, 0.0],
    }
}

/// A loop as kernel curves, with the sketch curve of each.
fn loop_curves(sketch: &Sketch, plane: Plane, l: &Loop) -> Result<Vec<(Curve, EntityId)>, String> {
    let point = |id: EntityId| {
        sketch
            .point(id)
            .map(|p| to_model(plane, p.x, p.y))
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
            let curves = loop_curves(&record.sketch, record.plane, l)?;
            origins.extend(curves.iter().map(|(_, e)| *e));
            loops.push(curves.into_iter().map(|(c, _)| c).collect::<Vec<_>>());
        }
        let n = u32::try_from(origins.len()).unwrap_or(u32::MAX);
        let made = kernel
            .extrude(&loops, normal(record.plane), *distance)
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
    for entry in document.features() {
        let outcome = (|| -> Result<Option<(Body, Names)>, String> {
            let Feature::Extrude { operation, .. } = &entry.feature;
            let (tool, names) = extrude_tool(document, kernel, entry.id, &entry.feature)?;
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
